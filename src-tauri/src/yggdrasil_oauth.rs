//! LittleSkin「通过 OAuth 访问 Yggdrasil API」：授权码流登录皮肤站账号。
//!
//! 传统 Yggdrasil 登录要启动器直接处理用户名/密码，无法支持二步验证与统一
//! 授权体验。LittleSkin 据此开放了 OAuth2：启动器走授权码流拿到 OAuth 访问
//! 令牌，再调两个新 API 把它换成可直接用于 Yggdrasil API 的 Minecraft 令牌，
//! 最终登记成一个普通的皮肤站（Yggdrasil）启动账号。
//!
//! 正式构建使用 LittleSkin 的公开设备代码流。角色签发完成后登记为皮肤站账号。
//! 开发构建可以使用个人测试客户端验证授权代码回调。

use crate::models::Account;
use crate::state::AppState;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 站点地址
pub const HOST: &str = "https://littleskin.cn";
/// Yggdrasil API root（登记账号时写入，令牌刷新/校验/纹理都走它）
pub const API_ROOT: &str = "https://littleskin.cn/api/yggdrasil";
/// 已在 LittleSkin 登记的回调地址（必须与登记值**完全一致**）
pub const REDIRECT_URI: &str = "qookix://oauth/littleskin/callback";
/// 授权范围：读角色档案 + 创建 Minecraft 令牌（空格分隔，见官方文档）
pub const SCOPE: &str = "Yggdrasil.PlayerProfiles.Read Yggdrasil.MinecraftToken.Create";
/// `state` 有效期（10 分钟）：够用户登录 + 点「允许」
const STATE_TTL: u64 = 600;

const DEVICE_CLIENT_ID: &str = match option_env!("LITTLESKIN_DEVICE_CLIENT_ID") {
    Some(value) => value,
    None => "",
};
const DEVICE_ORIGIN: &str = "https://open.littleskin.cn";

/// 登录结果里带回前端的角色（与 yggdrasil.rs 的 YggProfile 同形）
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YggOauthProfile {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YggOauthLinks {
    pub homepage: String,
    pub register: String,
}

/// `yggdrasil_oauth_complete` 的返回：角色列表 + 站点信息（前端据此展示角色选择）
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YggOauthCompleteResult {
    pub authorization_id: String,
    pub profiles: Vec<YggOauthProfile>,
    pub server: String,
    pub server_name: String,
    pub links: YggOauthLinks,
}

/// 进行中的 OAuth 流程状态：待校验的 state（落盘，扛冷启动回调）+ 换到的 OAuth
/// access_token（仅内存，选角色前临时持有）。
#[derive(Clone, Default)]
pub struct YggOauthPending {
    pending_state: Option<(String, u64)>,
    device_flow: bool,
    device: Option<DeviceGrant>,
    selection: Option<ProfileSelection>,
}

#[derive(Clone)]
struct DeviceGrant {
    code: String,
    client_id: String,
    interval: u64,
    poll_after: u64,
}

#[derive(Clone)]
struct ProfileSelection {
    authorization_id: String,
    oauth_token: String,
    expires_at: u64,
    profiles: Vec<YggOauthProfile>,
}

impl YggOauthPending {
    fn check_callback(&self, authorization_id: &str) -> Result<(), String> {
        matches!(&self.pending_state, Some((id, expires)) if id == authorization_id && now_secs() <= *expires)
            .then_some(()).ok_or_else(|| "LittleSkin 授权已变化或过期，请重新发起授权".into())
    }

    fn profile_selection(&self, authorization_id: &str, profile_id: &str) -> Result<&ProfileSelection, String> {
        self.selection.as_ref().filter(|selection| selection.authorization_id == authorization_id
            && now_secs() <= selection.expires_at
            && selection.profiles.iter().any(|p| p.id == profile_id))
            .ok_or_else(|| "LittleSkin 授权或所选角色已变化，请重新选择".into())
    }
}

fn pending_path(state: &AppState) -> PathBuf {
    state.root.join("ygg_oauth_pending.json")
}

/// 启动时从磁盘恢复待校验的 state（扛「用户在浏览器登录期间重启启动器」）。
pub fn load_saved(root: &std::path::Path) -> YggOauthPending {
    let text = match std::fs::read_to_string(root.join("ygg_oauth_pending.json")) {
        Ok(t) => t,
        Err(_) => return YggOauthPending::default(),
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return YggOauthPending::default();
    };
    YggOauthPending {
        pending_state: v
            .get("pending_state")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| {
                (
                    s.to_string(),
                    v.get("pending_expires").and_then(|e| e.as_u64()).unwrap_or(0),
                )
            }),
        device_flow: v.get("device_flow").and_then(Value::as_bool).unwrap_or(false),
        ..Default::default()
    }
}

fn save_pending(state: &AppState, pending: &YggOauthPending) -> Result<(), String> {
    if pending.pending_state.is_none() {
        return match std::fs::remove_file(pending_path(state)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("清理 LittleSkin 授权状态失败: {err}")),
        };
    }
    let payload = json!({
        "pending_state": pending.pending_state.as_ref().map(|(s, _)| s.clone()),
        "pending_expires": pending.pending_state.as_ref().map(|(_, e)| *e),
        "device_flow": pending.device_flow,
    });
    crate::fsutil::atomic_write(&pending_path(state), &serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| format!("保存 LittleSkin 授权状态失败: {e}"))
}

fn development_credentials(state: &AppState) -> Option<(String, String)> {
    if !cfg!(debug_assertions) { return None; }
    let settings = state.settings();
    let id = settings.littleskin_client_id.trim();
    let secret = settings.littleskin_client_secret.as_deref()?.trim();
    (!id.is_empty() && !secret.is_empty()).then(|| (id.to_owned(), secret.to_owned()))
}

fn now_secs() -> u64 {
    crate::util::now_secs()
}

/// 把 OAuth/token/角色接口的错误响应转成可读信息。
fn oauth_error(v: &Value, fallback: &str) -> String {
    let desc = v
        .get("error_description")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty());
    let code = v.get("error").and_then(|m| m.as_str()).filter(|s| !s.is_empty());
    match (code, desc) {
        (Some(c), Some(d)) => format!("{d}（{c}）"),
        (Some(c), None) => c.to_string(),
        (None, Some(d)) => d.to_string(),
        (None, None) => fallback.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 当前接入配置（登录窗口据此决定是否露出 OAuth2 入口）
#[tauri::command]
pub fn yggdrasil_oauth_config(app_state: tauri::State<'_, AppState>) -> Value {
    let dev = development_credentials(&app_state);
    let client_id = if cfg!(debug_assertions) {
        app_state.settings().littleskin_client_id.clone()
    } else {
        DEVICE_CLIENT_ID.to_owned()
    };
    json!({
        "host": HOST, "apiRoot": API_ROOT, "redirectUri": REDIRECT_URI, "scope": SCOPE,
        "clientId": client_id, "configured": !DEVICE_CLIENT_ID.trim().is_empty() || dev.is_some(),
        "secretConfigured": dev.is_some(), "devMode": cfg!(debug_assertions),
    })
}

#[tauri::command]
pub fn yggdrasil_set_credentials(
    app_state: tauri::State<'_, AppState>,
    client_id: String,
    client_secret: Option<String>,
) -> Result<Value, String> {
    if !cfg!(debug_assertions) {
        return Err("正式构建通过 LittleSkin 设备代码授权".into());
    }
    let mut patch = json!({ "littleskin_client_id": client_id });
    if let Some(secret) = client_secret.filter(|value| !value.trim().is_empty()) {
        patch["littleskin_client_secret"] = Value::String(secret);
    }
    crate::settings::update_settings(&app_state, patch)?;
    Ok(yggdrasil_oauth_config(app_state))
}

#[tauri::command]
pub async fn yggdrasil_oauth_start(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let public_id = DEVICE_CLIENT_ID.trim();
    let dev = development_credentials(&app_state);
    let device_flow = !public_id.is_empty();
    if !device_flow && dev.is_none() {
        return Err("LittleSkin 设备授权尚未配置".into());
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut pending = app_state.ygg_oauth.lock().unwrap();
        let next = YggOauthPending {
            pending_state: Some((id.clone(), now_secs() + STATE_TTL)),
            device_flow,
            ..Default::default()
        };
        save_pending(&app_state, &next)?;
        *pending = next;
    }
    if let Some((client_id, _)) = dev.filter(|_| !device_flow) {
        let url = format!(
            "{HOST}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={id}",
            utf8_percent_encode(&client_id, NON_ALPHANUMERIC),
            utf8_percent_encode(REDIRECT_URI, NON_ALPHANUMERIC),
            utf8_percent_encode(SCOPE, NON_ALPHANUMERIC),
        );
        return Ok(json!({ "mode": "callback", "url": url, "state": id, "redirectUri": REDIRECT_URI }));
    }

    let response = crate::settings::oauth_http_client(&app_state).post(format!("{DEVICE_ORIGIN}/oauth/device_code"))
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[("client_id", public_id), ("scope", SCOPE)])
        .send().await.map_err(|err| format!("获取 LittleSkin 授权码失败: {err}"))?;
    let status = response.status();
    let data = oauth_json(response).await?;
    if !status.is_success() {
        return Err(oauth_error(&data, "LittleSkin 拒绝申请设备代码"));
    }
    let code = data.get("device_code").and_then(Value::as_str).filter(|value| !value.is_empty())
        .ok_or("LittleSkin 未返回设备代码")?;
    let user_code = data.get("user_code").and_then(Value::as_str).filter(|value| !value.is_empty())
        .ok_or("LittleSkin 未返回用户授权码")?;
    let address = data.get("verification_uri_complete").or_else(|| data.get("verification_uri"))
        .and_then(Value::as_str).ok_or("LittleSkin 未返回验证地址")?;
    let mut url = reqwest::Url::parse(address).map_err(|_| "LittleSkin 验证地址无效")?;
    if url.scheme() != "https" || url.host_str() != Some("open.littleskin.cn") || url.path() != "/oauth/link"
        || !url.username().is_empty() || url.password().is_some()
    {
        return Err("LittleSkin 验证地址无效".into());
    }
    if !url.query_pairs().any(|(key, _)| key == "user_code") {
        url.query_pairs_mut().append_pair("user_code", user_code);
    }
    let interval = data.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, 60);
    let expires_in = data.get("expires_in").and_then(Value::as_u64).unwrap_or(300).clamp(1, STATE_TTL);
    let mut pending = app_state.ygg_oauth.lock().unwrap();
    pending.check_callback(&id)?;
    let mut next = pending.clone();
    next.pending_state = Some((id.clone(), now_secs() + expires_in));
    next.device = Some(DeviceGrant { code: code.to_owned(), client_id: public_id.to_owned(), interval, poll_after: now_secs() + interval });
    save_pending(&app_state, &next)?;
    *pending = next;
    Ok(json!({ "mode": "device", "url": url.as_str(), "state": id, "userCode": user_code, "interval": interval, "expiresIn": expires_in }))
}

async fn oauth_json(response: reqwest::Response) -> Result<Value, String> {
    let data = crate::util::read_response_limited(response, 128 * 1024).await?;
    serde_json::from_slice(&data).map_err(|_| "LittleSkin 返回了无效的授权数据".into())
}

#[tauri::command]
pub async fn yggdrasil_oauth_poll(app_state: tauri::State<'_, AppState>, state: String) -> Result<Value, String> {
    let lock = app_state.jobs.download_lock(app_state.root.join("littleskin-device-auth"));
    let Ok(_guard) = lock.try_lock() else { return Ok(json!({ "status": "pending" })); };
    let (code, client_id) = {
        let mut pending = app_state.ygg_oauth.lock().unwrap();
        pending.check_callback(&state)?;
        let device = pending.device.as_mut().ok_or("LittleSkin 设备授权已失效，请重新开始")?;
        if now_secs() < device.poll_after {
            return Ok(json!({ "status": "pending" }));
        }
        device.poll_after = now_secs() + device.interval;
        (device.code.clone(), device.client_id.clone())
    };
    let response = crate::settings::oauth_http_client(&app_state).post(format!("{DEVICE_ORIGIN}/oauth/token"))
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id.as_str()), ("device_code", code.as_str()),
        ]).send().await.map_err(|err| format!("查询设备授权失败: {err}"))?;
    let status = response.status();
    let data = oauth_json(response).await?;
    app_state.ygg_oauth.lock().unwrap().check_callback(&state)?;
    match data.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => return Ok(json!({ "status": "pending" })),
        Some("slow_down") => {
            let mut pending = app_state.ygg_oauth.lock().unwrap();
            pending.check_callback(&state)?;
            if let Some(device) = &mut pending.device {
                device.interval += 5;
                device.poll_after = now_secs() + device.interval;
            }
            return Ok(json!({ "status": "pending" }));
        }
        Some(_) => return Err(oauth_error(&data, "LittleSkin 设备授权失败")),
        None => {}
    }
    if !status.is_success() {
        return Err(oauth_error(&data, "LittleSkin 设备授权失败"));
    }
    let token = oauth_access(&data)?;
    let result = complete_with_token(&app_state, state, token).await?;
    Ok(json!({ "status": "ok", "result": result }))
}

#[tauri::command]
pub fn yggdrasil_oauth_cancel(app_state: tauri::State<'_, AppState>, state: String) -> Result<(), String> {
    let mut pending = app_state.ygg_oauth.lock().unwrap();
    let matches = pending.pending_state.as_ref().is_some_and(|(id, _)| id == &state)
        || pending.selection.as_ref().is_some_and(|flow| flow.authorization_id == state);
    if matches {
        let cleared = YggOauthPending::default();
        save_pending(&app_state, &cleared)?;
        *pending = cleared;
    }
    Ok(())
}

fn oauth_access(data: &Value) -> Result<String, String> {
    data.get("access_token").and_then(Value::as_str).filter(|value| !value.is_empty())
        .map(str::to_owned).ok_or_else(|| "LittleSkin 未返回访问令牌".into())
}

/// 第三步：用授权码换 OAuth access_token，并拉取用户名下全部角色。
/// OAuth token 临时记在内存里，等前端选定角色后由 `yggdrasil_oauth_add_account` 使用。
#[tauri::command]
pub async fn yggdrasil_oauth_complete(
    app_state: tauri::State<'_, AppState>,
    code: String,
    state: String,
) -> Result<YggOauthCompleteResult, String> {
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err("授权码为空".into());
    }
    // state 必须与发起时一致（防 CSRF / 回调来源校验）
    {
        let pending = app_state.ygg_oauth.lock().unwrap();
        pending.check_callback(&state)?;
        if pending.device_flow {
            return Err("LittleSkin 设备授权无需回调地址".into());
        }
    }
    let (client_id, client_secret) = development_credentials(&app_state).ok_or("此授权需要重新发起")?;
    let response = crate::settings::oauth_http_client(&app_state).post(format!("{HOST}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"), ("code", code.as_str()),
            ("client_id", client_id.as_str()), ("client_secret", client_secret.as_str()),
            ("redirect_uri", REDIRECT_URI),
        ]).send().await.map_err(|err| format!("换取访问令牌失败: {err}"))?;
    let status = response.status();
    let data = oauth_json(response).await?;
    if !status.is_success() {
        return Err(oauth_error(&data, "换取 LittleSkin 访问令牌失败"));
    }
    complete_with_token(&app_state, state, oauth_access(&data)?).await
}

async fn complete_with_token(app_state: &AppState, state: String, oauth_token: String)
    -> Result<YggOauthCompleteResult, String>
{
    app_state.ygg_oauth.lock().unwrap().check_callback(&state)?;
    // ---- 拉取用户名下全部角色档案 ----
    let resp = app_state
        .client
        .get(format!("{API_ROOT}/sessionserver/session/minecraft/profile"))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {oauth_token}"))
        .send()
        .await
        .map_err(|e| format!("拉取角色列表失败: {e}"))?;
    let pstatus = resp.status().as_u16();
    let ptext = resp.text().await.map_err(|e| e.to_string())?;
    if pstatus != 200 {
        let pv: Value = serde_json::from_str(&ptext).unwrap_or(Value::Null);
        return Err(oauth_error(&pv, &format!("拉取角色列表失败（HTTP {pstatus}）")));
    }
    let profiles_val: Value = serde_json::from_str(&ptext)
        .map_err(|e| format!("解析角色列表失败: {e}"))?;
    let profiles: Vec<YggOauthProfile> = profiles_val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    Some(YggOauthProfile {
                        id: p.get("id").and_then(|v| v.as_str())?.to_string(),
                        name: p.get("name").and_then(|v| v.as_str())?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if profiles.is_empty() {
        return Err("该账号下没有角色，请先到 LittleSkin 创建角色".into());
    }

    // 临时持有 OAuth token，清掉 pending_state（回调已用完）
    {
        let mut pending = app_state.ygg_oauth.lock().unwrap();
        pending.check_callback(&state)?;
        let next = YggOauthPending {
            selection: Some(ProfileSelection {
                authorization_id: state.clone(),
                oauth_token,
                expires_at: now_secs() + STATE_TTL,
                profiles: profiles.clone(),
            }),
            ..Default::default()
        };
        save_pending(&app_state, &next)?;
        *pending = next;
    }

    Ok(YggOauthCompleteResult {
        authorization_id: state,
        profiles,
        server: API_ROOT.to_string(),
        server_name: "LittleSkin".to_string(),
        links: YggOauthLinks {
            homepage: HOST.to_string(),
            register: format!("{HOST}/auth/register"),
        },
    })
}

/// 第四步：用 OAuth access_token 为选定角色签发 Minecraft 令牌，登记为皮肤站账号。
#[tauri::command]
pub async fn yggdrasil_oauth_add_account(
    app_state: tauri::State<'_, AppState>,
    profile_id: String,
    authorization_id: String,
) -> Result<Account, String> {
    let profile_id = profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Err("缺少角色信息".into());
    }
    let oauth_token = {
        let pending = app_state.ygg_oauth.lock().unwrap();
        pending.profile_selection(&authorization_id, &profile_id)?.oauth_token.clone()
    };

    // 调 LittleSkin 专属 API：用 OAuth token 换 Minecraft 令牌
    // 响应格式与 Yggdrasil 登录 API 一致：{ accessToken, clientToken, selectedProfile }
    let resp = app_state
        .client
        .post(format!("{API_ROOT}/authserver/oauth"))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {oauth_token}"))
        .json(&json!({ "uuid": profile_id }))
        .send()
        .await
        .map_err(|e| format!("获取 Minecraft 令牌失败: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|_| {
        format!("Minecraft 令牌接口返回非 JSON（HTTP {status}）：{}", text.chars().take(120).collect::<String>())
    })?;
    if status != 200 {
        // Yggdrasil 错误响应用 errorMessage 字段
        let msg = v
            .get("errorMessage")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("获取 Minecraft 令牌失败（HTTP {status}）"));
        return Err(msg);
    }
    let access_token = v
        .get("accessToken")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .ok_or("未返回 accessToken")?
        .to_string();
    let client_token = v
        .get("clientToken")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    // 优先用服务端回填的 selectedProfile，回退到请求的 profile_id
    let (pid, pname) = v
        .pointer("/selectedProfile/id")
        .and_then(|x| x.as_str())
        .and_then(|id| {
            let name = v.pointer("/selectedProfile/name").and_then(|x| x.as_str()).unwrap_or("");
            Some((id.to_string(), name.to_string()))
        })
        .unwrap_or_else(|| (profile_id.clone(), String::new()));
    if pname.is_empty() {
        return Err("服务端未返回角色名".into());
    }
    if !pid.eq_ignore_ascii_case(&profile_id) {
        return Err("LittleSkin 返回的角色与所选角色不一致".into());
    }

    let account = Account::Yggdrasil {
        uuid: pid,
        username: pname,
        created: now_secs(),
        access_token,
        client_token,
        server: API_ROOT.to_string(),
        server_name: "LittleSkin".to_string(),
    };
    let mut pending = app_state.ygg_oauth.lock().unwrap();
    pending.profile_selection(&authorization_id, &profile_id)?;
    crate::accounts::save_account(app_state.inner(), &account)?;
    pending.selection = None;
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_authorization_invalidates_old_callback_and_profile_selection() {
        let expires = now_secs() + 60;
        let pending = YggOauthPending { pending_state: Some(("new".into(), expires)), ..Default::default() };
        assert!(pending.check_callback("old").is_err());
        assert!(pending.check_callback("new").is_ok());
        let ready = YggOauthPending {
            selection: Some(ProfileSelection {
                authorization_id: "new".into(), oauth_token: "test-token".into(), expires_at: expires,
                profiles: vec![YggOauthProfile { id: "profile".into(), name: "Test".into() }],
            }),
            ..Default::default()
        };
        assert!(ready.check_callback("new").is_err());
        assert!(ready.profile_selection("old", "profile").is_err());
        assert!(ready.profile_selection("new", "unlisted").is_err());
        assert!(ready.profile_selection("new", "profile").is_ok());
        assert!(pending.profile_selection("new", "profile").is_err());
    }
}
