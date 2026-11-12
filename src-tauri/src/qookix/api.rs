//! 统一 API 层：信封解析、鉴权请求、令牌刷新闸门。
//!
//! 解析部分是纯函数（HTTP 状态码 + 响应体字符串 → 数据或错误），可以脱离
//! 网络做单元测试。服务端有两处例外必须照顾：
//! - 429 来自旧限流中间件，响应体是裸 `{"error":"rate limit exceeded"}`；
//! - 5xx 也可能是正常错误信封（`email_send_failed` / `jwt_unavailable`），
//!   所以不能先按状态码短路，必须先尝试解析 JSON。

use super::{clear_session, save_session, CODE_NOT_LOGGED_IN};
use crate::state::AppState;
use serde::Serialize;
use serde_json::{json, Value};

pub const BASE: &str = "https://service.qookix.cn";

/// 命令层统一错误：前端拿到的 reject 值就是这个结构
#[derive(Debug, Serialize)]
pub struct ApiError {
    /// HTTP 状态码，网络层失败或本地错误时为 0
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: u16, code: impl Into<String>, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn not_logged_in() -> Self {
        ApiError::new(0, CODE_NOT_LOGGED_IN, "尚未登录 QookiX 账号")
    }

    pub fn network(msg: impl Into<String>) -> Self {
        ApiError::new(0, "network_error", msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        ApiError::new(0, "internal_error", msg)
    }
}

/// 需要重新登录的码：拿到这些就清本地令牌
fn is_dead_token(code: &str) -> bool {
    matches!(
        code,
        "token_revoked" | "token_reused" | "invalid_token" | "unauthorized"
    )
}

fn truncate(body: &str, max: usize) -> String {
    let t = body.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut s: String = t.chars().take(max).collect();
    s.push('…');
    s
}

/// 把一次 HTTP 响应解析成业务数据。
pub fn decode(status: u16, body: &str) -> Result<Value, ApiError> {
    // 429 是旧限流中间件的裸响应，没有 ok/error 包装
    if status == 429 {
        return Err(ApiError::new(
            429,
            "rate_limited",
            "请求过于频繁，请稍后再试",
        ));
    }
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let Some(v) = parsed else {
        // 非 JSON：5xx 或是网关错误页，只能按状态码兜底
        let code = if status >= 500 {
            "server_error"
        } else {
            "invalid_response"
        };
        let msg = if body.trim().is_empty() {
            format!("服务端返回空响应（HTTP {status}）")
        } else {
            format!("服务端返回异常（HTTP {status}）：{}", truncate(body, 160))
        };
        return Err(ApiError::new(status, code, msg));
    };

    if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
        return Ok(v.get("data").cloned().unwrap_or(Value::Null));
    }

    let err = v.get("error");
    let code = err
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .unwrap_or(if status >= 500 {
            "server_error"
        } else {
            "unknown_error"
        });
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or("请求失败");
    Err(ApiError::new(status, code, message))
}

async fn send_raw(
    state: &AppState,
    method: reqwest::Method,
    path: &str,
    query: &[(String, String)],
    body: Option<&Value>,
    token: Option<&str>,
) -> Result<Value, ApiError> {
    let mut req = state
        .client
        .request(method, format!("{BASE}{path}"))
        .header(reqwest::header::ACCEPT, "application/json");
    if !query.is_empty() {
        req = req.query(query);
    }
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    // 服务端对 POST/PATCH 强校验 Content-Type，无 body 的接口传 {} 最省事
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ApiError::network(format!("请求 QookiX 服务失败: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError::network(format!("读取 QookiX 服务响应失败: {e}")))?;
    decode(status, &text)
}

/// 公开接口（不带令牌）
pub async fn post_public(
    state: &AppState,
    path: &str,
    body: Value,
) -> Result<Value, ApiError> {
    send_raw(state, reqwest::Method::POST, path, &[], Some(&body), None).await
}

/// 鉴权 GET
pub async fn get(
    state: &AppState,
    path: &str,
    query: Vec<(String, String)>,
) -> Result<Value, ApiError> {
    authed(state, reqwest::Method::GET, path, query, None).await
}

/// 鉴权 POST（无请求体的接口传 `json!({})`）
pub async fn post(state: &AppState, path: &str, body: Value) -> Result<Value, ApiError> {
    authed(state, reqwest::Method::POST, path, Vec::new(), Some(body)).await
}

/// 鉴权 PATCH
pub async fn patch(state: &AppState, path: &str, body: Value) -> Result<Value, ApiError> {
    authed(state, reqwest::Method::PATCH, path, Vec::new(), Some(body)).await
}

/// 鉴权 PUT（配置云同步用）
pub async fn put(state: &AppState, path: &str, body: Value) -> Result<Value, ApiError> {
    authed(state, reqwest::Method::PUT, path, Vec::new(), Some(body)).await
}

/// 鉴权 DELETE（`/blocks` 用 query 传参）
pub async fn delete(
    state: &AppState,
    path: &str,
    query: Vec<(String, String)>,
) -> Result<Value, ApiError> {
    authed(state, reqwest::Method::DELETE, path, query, None).await
}

async fn authed(
    state: &AppState,
    method: reqwest::Method,
    path: &str,
    query: Vec<(String, String)>,
    body: Option<Value>,
) -> Result<Value, ApiError> {
    let token = ensure_fresh(state).await?;
    let first = send_raw(state, method.clone(), path, &query, body.as_ref(), Some(&token)).await;
    match first {
        Ok(v) => Ok(v),
        // access token 到期：刷新一次再重放
        Err(e) if e.status == 401 && e.code == "token_expired" => {
            let token = refresh(state, &token).await?;
            send_raw(state, method, path, &query, body.as_ref(), Some(&token)).await
        }
        Err(e) if e.status == 401 && is_dead_token(&e.code) => {
            clear_session(state);
            Err(ApiError::not_logged_in())
        }
        Err(e) => Err(e),
    }
}

/// 取一个可用的 access token：快过期就先刷新
async fn ensure_fresh(state: &AppState) -> Result<String, ApiError> {
    let (token, expires_at) = {
        let s = state.qookix();
        (s.access_token.clone(), s.expires_at)
    };
    if token.is_empty() {
        return Err(ApiError::not_logged_in());
    }
    // 留 60 秒余量，避免请求发到一半正好过期
    if expires_at == 0 || crate::util::now_secs() + 60 < expires_at {
        return Ok(token);
    }
    refresh(state, &token).await
}

/// 刷新访问令牌。
///
/// 闸门内要重新读一次当前令牌：多个请求同时撞上 401 时，只有第一个真的去
/// 刷新，其余的等它完成后直接用新令牌——refresh token 一次性使用，重复提交
/// 会让后来者拿到 `token_reused` 并被强制登出。
async fn refresh(state: &AppState, stale: &str) -> Result<String, ApiError> {
    let _gate = state.qookix_refresh.lock().await;
    let (cur, cur_exp, refresh_token) = {
        let s = state.qookix();
        (s.access_token.clone(), s.expires_at, s.refresh_token.clone())
    };
    if !cur.is_empty()
        && cur != stale
        && (cur_exp == 0 || crate::util::now_secs() + 30 < cur_exp)
    {
        return Ok(cur);
    }
    if refresh_token.is_empty() {
        return Err(ApiError::not_logged_in());
    }
    let data = post_public(
        state,
        "/api/v1/auth/refresh",
        json!({ "refresh_token": refresh_token }),
    )
    .await
    .map_err(|e| {
        if is_dead_token(&e.code) {
            clear_session(state);
            ApiError::not_logged_in()
        } else {
            e
        }
    })?;
    let tokens = data
        .get("tokens")
        .ok_or_else(|| ApiError::internal("刷新令牌时服务端未返回 tokens"))?;
    apply_tokens(state, tokens)?;
    let s = state.qookix();
    Ok(s.access_token.clone())
}

/// 把服务端返回的 tokens 写入会话并落盘
pub fn apply_tokens(state: &AppState, tokens: &Value) -> Result<(), ApiError> {
    let access = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if access.is_empty() {
        return Err(ApiError::internal("服务端返回的 access_token 为空"));
    }
    let refresh = tokens
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let ttl = tokens
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(7200);
    {
        let mut s = state.qookix();
        s.access_token = access.to_string();
        if !refresh.is_empty() {
            s.refresh_token = refresh.to_string();
        }
        s.expires_at = crate::util::now_secs() + ttl;
    }
    save_session(state).map_err(ApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope_unwraps_data() {
        let v = decode(200, r#"{"ok":true,"data":{"a":1}}"#).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn success_without_data_is_null() {
        let v = decode(200, r#"{"ok":true}"#).unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn error_envelope_uses_code() {
        let e = decode(401, r#"{"ok":false,"error":{"code":"token_expired","message":"过期"}}"#).unwrap_err();
        assert_eq!(e.status, 401);
        assert_eq!(e.code, "token_expired");
        assert_eq!(e.message, "过期");
    }

    /// 5xx 也可能带正常信封：不能先按状态码短路，否则丢失业务码。
    #[test]
    fn server_error_with_envelope_keeps_code() {
        let e = decode(503, r#"{"ok":false,"error":{"code":"email_send_failed","message":"验证邮件发送失败"}}"#).unwrap_err();
        assert_eq!(e.status, 503);
        assert_eq!(e.code, "email_send_failed");
    }

    /// 429 是裸响应，且必须先于 JSON 解析判定
    #[test]
    fn rate_limit_is_detected_before_envelope() {
        let e = decode(429, r#"{"error":"rate limit exceeded"}"#).unwrap_err();
        assert_eq!(e.code, "rate_limited");
    }

    #[test]
    fn non_json_5xx_falls_back_to_server_error() {
        let e = decode(502, "<html>bad gateway</html>").unwrap_err();
        assert_eq!(e.code, "server_error");
        assert_eq!(e.status, 502);
        assert!(e.message.contains("502"));
    }

    #[test]
    fn empty_body_reports_status() {
        let e = decode(500, "").unwrap_err();
        assert_eq!(e.code, "server_error");
        assert!(e.message.contains("空响应"));
    }

    #[test]
    fn dead_token_codes_are_recognised() {
        for c in ["token_revoked", "token_reused", "invalid_token", "unauthorized"] {
            assert!(is_dead_token(c), "{c} 应当触发重新登录");
        }
        assert!(!is_dead_token("token_expired"));
        assert!(!is_dead_token("not_found"));
    }
}
