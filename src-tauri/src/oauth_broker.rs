use crate::state::AppState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SERVICE: &str = "https://service.qookix.cn";

fn broker_base(path: &str) -> Option<String> {
    let path = path.trim().trim_end_matches('/');
    let relative = path.strip_prefix('/')?;
    if relative.is_empty()
        || !relative.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        return None;
    }
    Some(format!("{SERVICE}{path}"))
}

pub fn shimu_broker() -> Option<String> {
    broker_base(option_env!("SHIMU_OAUTH_BROKER_PATH")?)
}

pub fn verifier() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

async fn post(
    state: &AppState,
    base: &str,
    operation: &str,
    value: Value,
) -> Result<Value, String> {
    let response = crate::settings::oauth_http_client(state)
        .post(format!("{base}/{operation}"))
        .json(&value)
        .send()
        .await
        .map_err(|err| format!("连接拾木空间授权服务失败: {err}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "拾木空间授权服务返回 HTTP {}，请重新授权",
            response.status()
        ));
    }
    let bytes = crate::util::read_response_limited(response, 128 * 1024).await?;
    serde_json::from_slice(&bytes).map_err(|_| "授权服务响应格式错误".into())
}

pub async fn start(
    state: &AppState,
    base: &str,
    id: &str,
    verifier: &str,
) -> Result<String, String> {
    let data = post(
        state,
        base,
        "start",
        json!({
            "state": id, "code_challenge": challenge(verifier), "code_challenge_method": "S256",
        }),
    )
    .await?;
    let raw = data
        .get("authorization_url")
        .and_then(Value::as_str)
        .ok_or("授权服务未返回授权网址")?;
    let url = Url::parse(raw).map_err(|_| "授权网址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.mcimg.com")
        || url.path() != "/oauth/authorize"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("授权服务返回的拾木空间网址无效".into());
    }
    Ok(url.to_string())
}

pub async fn complete(
    state: &AppState,
    base: &str,
    id: &str,
    handoff: &str,
    verifier: &str,
) -> Result<Value, String> {
    if handoff.is_empty() || handoff.len() > 512 {
        return Err("授权交接码无效".into());
    }
    post(
        state,
        base,
        "complete",
        json!({ "state": id, "handoff": handoff, "code_verifier": verifier }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_uses_s256_and_broker_is_confined_to_service_origin() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_ne!(verifier(), verifier());
        assert_eq!(
            broker_base("/api/v1/desktop/shimu").as_deref(),
            Some("https://service.qookix.cn/api/v1/desktop/shimu")
        );
        assert!(broker_base("//other.example/api").is_none());
        assert!(broker_base("/api/../other").is_none());
    }
}
