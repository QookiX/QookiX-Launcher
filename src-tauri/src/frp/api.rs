//! 内网穿透（frp）面板 API 客户端。
//!
//! 对接 MohuaFrp 面板：设备授权登录 → 隧道列表 → 服务端下发 frpc 配置。
//! 三条来自官方对接文档的硬要求，别改：
//! 1. 强制 HTTP/1.1（官网 WAF 会把 HTTP/2 判定成异常请求）；
//! 2. Token 轮询每轮**先等 interval 再请求**（注册与轮询共用限流桶）；
//! 3. 设备注册每分钟最多 5 次，命中限流要等满 60 秒。

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const BASE: &str = "https://frp.mhjz1.cn";

/// 通用响应包装
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
    // Option 本身对缺字段就是 None；再加 serde(default) 会平白引入 T: Default 约束
    data: Option<T>,
}

/// 复用同一个客户端：HTTP/1.1 + 20 秒超时
fn client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .http1_only()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 把错误响应转成一句人话
fn envelope_err(status: u16, body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        for k in ["message", "error"] {
            if let Some(s) = v.get(k).and_then(|x| x.as_str()) {
                if !s.trim().is_empty() {
                    return s.trim().to_string();
                }
            }
        }
    }
    let snip: String = body.trim().chars().take(120).collect();
    format!("HTTP {status}: {snip}")
}

/// 解包 `{status,message,error,data}`
fn unwrap_data<T: for<'de> Deserialize<'de>>(status: u16, body: &str) -> Result<T, String> {
    if status >= 400 {
        return Err(envelope_err(status, body));
    }
    let env: Envelope<T> = serde_json::from_str(body).map_err(|e| format!("面板响应解析失败: {e}"))?;
    if env.status == "error" {
        return Err(if env.message.is_empty() {
            env.error
        } else {
            env.message
        });
    }
    env.data.ok_or_else(|| "面板未返回数据".to_string())
}

// ------------------------------------------------------------------ 设备授权

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuth {
    pub device_code: String,
    /// 展示给用户的授权码，如 `WDJB-MJHT`
    pub user_code: String,
    /// 已带 user_code 的完整授权链接（优先用它）
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// 第一步：设备注册，拿到用户授权码
pub async fn device_register() -> Result<DeviceAuth, String> {
    let resp = client()
        .post(format!("{BASE}/auth/device_register.php"))
        .header("User-Agent", "QookiX-Launcher/1.0.0 (windows)")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await
        .map_err(|e| format!("请求面板失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let mut data: DeviceAuth = unwrap_data(status, &body)?;
    // 服务端可能不给完整链接，自己拼一个
    if data.verification_uri_complete.is_empty() {
        data.verification_uri_complete =
            format!("{BASE}/device?user_code={}", data.user_code);
    }
    data.interval = data.interval.max(5);
    if data.expires_in == 0 {
        data.expires_in = 300;
    }
    Ok(data)
}

/// 轮询结果
#[derive(Debug, Clone)]
pub enum PollResult {
    Pending,
    Authorized(String),
}

#[derive(Debug, Deserialize)]
struct TokenResp {
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    token: String,
}

/// 第二步：轮询一次（**调用方负责先 sleep interval**）
pub async fn device_token(device_code: &str) -> Result<PollResult, String> {
    let resp = client()
        .post(format!("{BASE}/auth/device_token.php"))
        .header("User-Agent", "QookiX-Launcher/1.0.0 (windows)")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[("device_code", device_code)])
        .send()
        .await
        .map_err(|e| format!("请求面板失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if status >= 400 {
        return Err(envelope_err(status, &body));
    }
    // 这个接口**不是**通用包装，直接是扁平结构
    let t: TokenResp =
        serde_json::from_str(&body).map_err(|e| format!("授权响应解析失败: {e}"))?;
    match t.status.as_str() {
        "pending" => Ok(PollResult::Pending),
        "authorized" => {
            if t.token.trim().is_empty() {
                return Err("授权成功但未返回 Token".into());
            }
            Ok(PollResult::Authorized(t.token))
        }
        "error" => {
            let msg = if t.message.is_empty() { t.error.clone() } else { t.message };
            Err(match t.error.as_str() {
                "expired_token" => "授权码已过期，请重新获取".to_string(),
                "access_denied" => "已取消授权".to_string(),
                _ => msg,
            })
        }
        // 未知状态按继续等待处理，避免偶发字段变化直接把用户踢出去
        _ => Ok(PollResult::Pending),
    }
}

// ------------------------------------------------------------------ 账号与隧道

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrpUser {
    pub id: i64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub balance: f64,
    #[serde(default)]
    pub tunnel_stats: Option<serde_json::Value>,
}

/// 账号信息（也用作登录态校验）
pub async fn user_balance(token: &str) -> Result<FrpUser, String> {
    let resp = client()
        .get(format!("{BASE}/proxy/user_balance.php"))
        .bearer_auth(token)
        .header("User-Agent", "QookiX-Launcher/1.0.0 (windows)")
        .send()
        .await
        .map_err(|e| format!("请求面板失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    unwrap_data(status, &body)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tunnel {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub local_ip: String,
    #[serde(default)]
    pub local_port: u32,
    #[serde(default)]
    pub remote_port: u32,
    #[serde(default)]
    pub custom_domain: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub node_name: String,
    #[serde(default)]
    pub node_hostname: String,
    #[serde(default)]
    pub node_port: u32,
    #[serde(default)]
    pub billing_type: String,
}

#[derive(Debug, Deserialize)]
struct TunnelList {
    #[serde(default)]
    tunnels: Vec<Tunnel>,
}

pub async fn list_tunnels(token: &str) -> Result<Vec<Tunnel>, String> {
    let resp = client()
        .get(format!("{BASE}/proxy/list.php?page=1&pageSize=100"))
        .bearer_auth(token)
        .header("User-Agent", "QookiX-Launcher/1.0.0 (windows)")
        .send()
        .await
        .map_err(|e| format!("请求面板失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let list: TunnelList = unwrap_data(status, &body)?;
    Ok(list.tunnels)
}

#[derive(Debug, Clone, Deserialize)]
pub struct TunnelConfig {
    /// 服务端拼好的完整 frpc TOML，已带认证 Token
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub tunnel_name: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u32,
}

pub async fn tunnel_config(token: &str, tunnel_id: i64) -> Result<TunnelConfig, String> {
    let resp = client()
        .get(format!(
            "{BASE}/frps/frpc_download.php?tunnel_id={tunnel_id}&reset_key=0&format=json"
        ))
        .bearer_auth(token)
        .header("User-Agent", "QookiX-Launcher/1.0.0 (windows)")
        .send()
        .await
        .map_err(|e| format!("请求面板失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    unwrap_data(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_error_prefers_message() {
        let body = r#"{"status":"error","message":"请求过于频繁，请稍后再试","error":"rate"}"#;
        assert!(envelope_err(200, body).contains("频繁"));
    }

    #[test]
    fn unwrap_data_rejects_error_status() {
        let body = r#"{"status":"error","message":"隧道不存在","data":null}"#;
        let r: Result<serde_json::Value, _> = unwrap_data(200, body);
        assert_eq!(r.unwrap_err(), "隧道不存在");
    }

    #[test]
    fn unwrap_data_returns_payload() {
        let body = r#"{"status":"success","data":{"id":7,"username":"alice"}}"#;
        let u: FrpUser = unwrap_data(200, body).unwrap();
        assert_eq!(u.id, 7);
        assert_eq!(u.username, "alice");
    }

    #[test]
    fn list_parses_tunnels() {
        let body = r#"{"status":"success","data":{"tunnels":[
            {"id":63,"name":"mc","protocol":"tcp","local_port":25565,"remote_port":30001,"status":"active","node_hostname":"node1.frp.mhjz1.cn","node_port":7000}
        ]}}"#;
        let l: TunnelList = unwrap_data(200, body).unwrap();
        assert_eq!(l.tunnels.len(), 1);
        assert_eq!(l.tunnels[0].name, "mc");
        assert_eq!(l.tunnels[0].node_port, 7000);
    }
}
