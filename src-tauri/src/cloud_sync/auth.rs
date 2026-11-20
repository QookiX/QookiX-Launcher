//! GitHub OAuth Device Flow：授权、令牌刷新与断连。
//! Client ID 为公开信息，硬编码安全（无 Client Secret）。

use super::github;
use super::store::{self, CloudState};
use crate::state::AppState;
use serde_json::json;
use std::sync::Mutex;
use uuid::Uuid;

const CLIENT_ID: &str = "Ov23liANpccEHSzQmJs1";

#[derive(Default)]
struct DeviceFlow {
    generation: Option<Uuid>,
    code: String,
}

impl DeviceFlow {
    fn check(&self, generation: Uuid) -> Result<(), String> {
        (self.generation == Some(generation))
            .then_some(())
            .ok_or_else(|| "GitHub 设备授权已变化，请重新开始".into())
    }
}

static DEVICE_FLOW: Mutex<DeviceFlow> = Mutex::new(DeviceFlow { generation: None, code: String::new() });

fn parse_account(v: &serde_json::Value) -> Result<String, String> {
    v.get("login")
        .and_then(|v| v.as_str())
        .filter(|login| !login.is_empty() && login.len() <= 100
            && login.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
        .map(str::to_owned)
        .ok_or_else(|| "GitHub 返回了无效的账号名，请重新授权".into())
}

fn expires_at(response: &serde_json::Value) -> u64 {
    response.get("expires_in").and_then(|v| v.as_u64())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| now_secs().saturating_add(seconds))
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 设备流程第一步：申请设备码与用户码。
pub async fn start_device_flow(state: &AppState) -> Result<serde_json::Value, String> {
    let generation = Uuid::new_v4();
    *DEVICE_FLOW.lock().map_err(|_| "无法锁定设备授权")? = DeviceFlow {
        generation: Some(generation),
        code: String::new(),
    };
    let resp = state
        .client
        .post("https://github.com/login/device/code")
        .header("User-Agent", github::UA)
        .header("Accept", "application/json")
        .form(&[("client_id", CLIENT_ID), ("scope", "repo")])
        .send()
        .await
        .map_err(|e| format!("请求设备码失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析设备码响应失败: {e}"))?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return Err(format!("申请设备码失败: {err}"));
    }
    let code = v.get("device_code").and_then(|x| x.as_str()).filter(|s| !s.is_empty())
        .ok_or("GitHub 未返回设备码")?;
    let user_code = v.get("user_code").and_then(|x| x.as_str()).filter(|s| !s.is_empty())
        .ok_or("GitHub 未返回用户码")?;
    let mut flow = DEVICE_FLOW.lock().map_err(|_| "无法锁定设备授权")?;
    flow.check(generation)?;
    flow.code = code.to_owned();
    Ok(json!({
        "deviceCode": code,
        "userCode": user_code,
        "verificationUri": v.get("verification_uri").and_then(|x| x.as_str()).unwrap_or("https://github.com/login/device"),
        "interval": v.get("interval").and_then(|x| x.as_u64()).unwrap_or(5),
        "expiresIn": v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(900),
    }))
}

/// 设备流程第二步：轮询换取令牌。返回 `{ status: "pending" | "ok", ... }`。
pub async fn poll_device_flow(state: &AppState, device_code: &str) -> Result<serde_json::Value, String> {
    let generation = {
        let flow = DEVICE_FLOW.lock().map_err(|_| "无法锁定设备授权")?;
        flow.generation.filter(|_| !flow.code.is_empty() && flow.code == device_code)
            .ok_or("GitHub 设备授权已变化，请重新开始")?
    };
    let resp = state
        .client
        .post("https://github.com/login/oauth/access_token")
        .header("User-Agent", github::UA)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", CLIENT_ID),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("轮询令牌失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析令牌响应失败: {e}"))?;
    DEVICE_FLOW.lock().map_err(|_| "无法锁定设备授权")?.check(generation)?;
    if let Some(access) = v.get("access_token").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
        let account = fetch_account(state, access).await?;
        let mut flow = DEVICE_FLOW.lock().map_err(|_| "无法锁定设备授权")?;
        flow.check(generation)?;
        let access = crate::secret::protect(access)?;
        let refresh = crate::secret::protect(
            v.get("refresh_token").and_then(|x| x.as_str()).unwrap_or(""),
        )?;
        store::edit(state, |cs| {
            if !cs.account.is_empty() && !cs.account.eq_ignore_ascii_case(&account) {
                cs.repo_name.clear();
                cs.repository_id.clear();
                cs.worlds.clear();
            }
            cs.session_id = generation.to_string();
            cs.account.clone_from(&account);
            cs.access_token = access;
            cs.refresh_token = refresh;
            cs.expires_at = expires_at(&v);
            Ok(())
        })?;
        *flow = DeviceFlow::default();
        return Ok(json!({ "status": "ok", "account": account }));
    }
    let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("unknown");
    match err {
        "authorization_pending" | "slow_down" => Ok(json!({ "status": "pending" })),
        "expired_token" => Err("用户码已过期，请重新开始授权".into()),
        "access_denied" => Err("用户拒绝了授权".into()),
        other => Err(format!("授权失败: {other}")),
    }
}

async fn fetch_account(state: &AppState, token: &str) -> Result<String, String> {
    let Some(v) = github::get(&state.client, &format!("{}/user", github::API), token).await? else {
        return Err("获取账号信息失败".into());
    };
    parse_account(&v)
}

/// 确保 access_token 未过期；过期则用 refresh_token 换新（轮换后必须保存新的）。
pub async fn ensure_context(state: &AppState) -> Result<(String, CloudState), String> {
    let expected = store::load(state);
    let lock = state.jobs.download_lock(state.root.join("cloud_sync").join(".refresh"));
    let _refresh = lock.lock().await;
    let cs = store::load(state);
    cs.check_session(&expected)?;
    let token = store::access_token(&cs);
    if token.is_empty() || !cs.connected() {
        return Err("尚未连接 GitHub，请先授权".into());
    }
    // 有效期留 5 分钟余量；expires_at == 0 表示令牌不过期（未启用过期选项）
    if cs.expires_at == 0 || now_secs() + 300 < cs.expires_at {
        return Ok((token, cs));
    }
    let refresh = store::decrypt_secret(&cs.refresh_token);
    if refresh.is_empty() {
        return Err("令牌已过期且无刷新令牌，请重新授权".into());
    }
    let resp = state
        .client
        .post("https://github.com/login/oauth/access_token")
        .header("User-Agent", github::UA)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("刷新令牌失败: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析刷新响应失败: {e}"))?;
    let Some(access) = v.get("access_token").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) else {
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("unknown");
        return Err(format!("刷新令牌失败: {err}（请重新授权）"));
    };
    let result = store::edit(state, |current| {
        current.check_session(&cs)?;
        current.access_token = crate::secret::protect(access)?;
        if let Some(rt) = v.get("refresh_token").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
            current.refresh_token = crate::secret::protect(rt)?;
        }
        current.expires_at = expires_at(&v);
        Ok(current.clone())
    })?;
    Ok((access.to_owned(), result))
}

/// 断开连接：清空令牌与仓库信息（云端仓库与数据不动，由用户自行处置）。
pub fn disconnect(state: &AppState) -> Result<(), String> {
    let mut flow = DEVICE_FLOW.lock().map_err(|_| "无法锁定设备授权")?;
    *flow = DeviceFlow::default();
    store::edit(state, |cs| {
        *cs = CloudState { keep_per_world: cs.keep_per_world, ..Default::default() };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_account_before_publishing_device_credentials() {
        assert!(parse_account(&json!({})).is_err());
        assert!(parse_account(&json!({ "login": "" })).is_err());
        assert_eq!(parse_account(&json!({ "login": "example-user" })).unwrap(), "example-user");
    }

    #[test]
    fn disconnected_or_replaced_device_flow_rejects_old_response() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let flow = DeviceFlow { generation: Some(second), code: "new-code".into() };
        assert!(flow.check(first).is_err());
        assert!(flow.check(second).is_ok());
        assert!(DeviceFlow::default().check(second).is_err());
    }
}
