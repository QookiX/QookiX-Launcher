//! 拾木空间（www.mcimg.com）第三方应用接入：OAuth 授权登录 + 开放 API。
//!
//! ⚠️ 合规边界：拾木空间账号**不是启动账号**。它的用户信息里带 `mcUuid`，
//! 但那只作资料展示/识别用 —— 绝不能拿它去充当启动身份或绕过 Minecraft
//! 正版验证（Minecraft 使用准则禁止规避正版验证）。启动游戏仍走启动器
//! 既有的微软 / 离线 / 皮肤站账号体系。
//!
//! 正式构建通过 QookiX 服务中转授权码，桌面端只持有一次性交接码及验证值。
//! 已保存的用户令牌仍按原方式访问拾木空间 API，过期后重新授权。

use crate::state::AppState;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 站点地址
pub const HOST: &str = "https://www.mcimg.com";
/// API 基础地址（所有 `/api/**` 前加 `/shimu`）
pub const BASE: &str = "https://www.mcimg.com/shimu";
/// 已在拾木空间登记的回调地址（必须与登记值**完全一致**）
pub const REDIRECT_URI: &str = "qookix://oauth/callback";
/// 授权范围
pub const SCOPE: &str = "user:info,post:read,file:download";
/// 服务端没返回 `expires_in` 时的兜底有效期（3 天）
const DEFAULT_EXPIRES_IN: u64 = 259_200;
/// `state` 有效期（10 分钟）：够用户登录 + 点「允许」
const STATE_TTL: u64 = 600;
/// 下载体积上限（512 MiB）：投影文件远小于此，防误点大文件把内存吃满
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

/// 未登录（前端据此打开登录窗口）
pub const ERR_NOT_LOGGED_IN: &str = "__shimu_not_logged_in__";
/// token 过期 / 被服务端拒绝（前端据此引导重新授权）
pub const ERR_EXPIRED: &str = "__shimu_expired__";

// ---------------------------------------------------------------------------
// 登录态
// ---------------------------------------------------------------------------

/// 拾木空间登录态：一次性授权码流程的 `state` + 当前 token + 用户资料。
#[derive(Clone, Default)]
pub struct ShimuAuth {
    generation: uuid::Uuid,
    broker_verifier: Option<String>,
    /// 待回调校验的 (state, 过期时间戳)
    pub pending_state: Option<(String, u64)>,
    pub access_token: Option<String>,
    /// token 过期时间戳（0 = 未知，按不限期处理）
    pub expires_at: u64,
    /// `getInfo` 返回的用户对象（原样保存，字段随服务端升级自动跟上）
    pub user: Option<Value>,
}

struct RequestAuth {
    generation: uuid::Uuid,
    token: String,
}

impl ShimuAuth {
    fn check_callback(&self, state: &str) -> Result<(), String> {
        matches!(&self.pending_state, Some((id, expires)) if id == state && crate::util::now_secs() <= *expires)
            .then_some(()).ok_or_else(|| "拾木空间授权已变化或过期，请重新发起授权".into())
    }

    fn check_request(&self, request: &RequestAuth) -> Result<(), String> {
        (self.generation == request.generation && self.access_token.as_deref() == Some(&request.token))
            .then_some(()).ok_or_else(|| "拾木空间会话已变化，请重试".into())
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn old_profiles_and_callbacks_cannot_change_current_auth() {
        let now = crate::util::now_secs();
        let current = ShimuAuth {
            generation: uuid::Uuid::new_v4(),
            pending_state: Some(("new-state".into(), now + 60)),
            access_token: Some("token".into()),
            ..Default::default()
        };
        let request = RequestAuth { generation: current.generation, token: "token".into() };
        let older = RequestAuth { generation: uuid::Uuid::new_v4(), token: "token".into() };
        assert!(current.check_request(&request).is_ok());
        assert!(current.check_request(&older).is_err());
        assert!(ShimuAuth::default().check_request(&request).is_err());
        assert!(current.check_callback("old-state").is_err());
        assert!(current.check_callback("new-state").is_ok());
        assert!(ShimuAuth::default().check_callback("new-state").is_err());
    }
}

fn auth_path(state: &AppState) -> PathBuf {
    state.root.join("shimu_auth.json")
}

/// 启动时从磁盘恢复登录态（token 解密失败视为未登录）。
pub fn load_saved(root: &std::path::Path) -> ShimuAuth {
    let text = match std::fs::read_to_string(root.join("shimu_auth.json")) {
        Ok(t) => t,
        Err(_) => return ShimuAuth::default(),
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return ShimuAuth::default();
    };
    let token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .and_then(|t| crate::secret::unprotect(t).ok())
        .unwrap_or_default();
    ShimuAuth {
        generation: uuid::Uuid::new_v4(),
        broker_verifier: v.get("broker_verifier").and_then(Value::as_str)
            .and_then(|encoded| crate::secret::unprotect(encoded).ok()),
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
        access_token: if token.is_empty() { None } else { Some(token) },
        expires_at: v.get("expires_at").and_then(|e| e.as_u64()).unwrap_or(0),
        user: v.get("user").cloned().filter(|u| !u.is_null()),
    }
}

fn save_auth(state: &AppState, auth: &ShimuAuth) -> Result<(), String> {
    let token = auth.access_token.as_deref().filter(|t| !t.is_empty());
    if token.is_none() && auth.pending_state.is_none() {
        return match std::fs::remove_file(auth_path(state)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("清除拾木空间登录状态失败: {err}")),
        };
    }
    let payload = json!({
        "access_token": match token {
            Some(t) => crate::secret::protect(t)?,
            None => String::new(),
        },
        "broker_verifier": auth.broker_verifier.as_deref().map(crate::secret::protect).transpose()?,
        "expires_at": auth.expires_at,
        "pending_state": auth.pending_state.as_ref().map(|(s, _)| s.clone()),
        "pending_expires": auth.pending_state.as_ref().map(|(_, e)| *e),
        "user": auth.user.clone().unwrap_or(Value::Null),
    });
    crate::fsutil::atomic_write(
        &auth_path(state),
        &serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("保存拾木空间登录状态失败: {e}"))
}

fn clear_session(state: &AppState, request: Option<&RequestAuth>) -> Result<(), String> {
    let mut auth = state.shimu.lock().unwrap();
    let pending_state = if let Some(request) = request {
        auth.check_request(request)?;
        auth.pending_state.clone()
    } else {
        None
    };
    let broker_verifier = pending_state.as_ref().and(auth.broker_verifier.clone());
    let next = ShimuAuth { generation: uuid::Uuid::new_v4(), pending_state, broker_verifier, ..Default::default() };
    save_auth(state, &next)?;
    *auth = next;
    Ok(())
}

fn development_credentials(state: &AppState) -> Option<(String, String)> {
    if !cfg!(debug_assertions) { return None; }
    let settings = state.settings();
    let id = settings.shimu_client_id.trim();
    let secret = settings.shimu_client_secret.as_deref()?.trim();
    (!id.is_empty() && !secret.is_empty()).then(|| (id.to_owned(), secret.to_owned()))
}

fn current_token(state: &AppState) -> Result<RequestAuth, String> {
    let auth = state.shimu.lock().unwrap();
    let token = auth
        .access_token
        .as_deref()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ERR_NOT_LOGGED_IN.to_string())?;
    if auth.expires_at > 0 && crate::util::now_secs() >= auth.expires_at {
        return Err(ERR_EXPIRED.to_string());
    }
    Ok(RequestAuth { generation: auth.generation, token: token.to_owned() })
}

/// 可选登录：帖子列表/详情等接口**匿名就能看**，登录后才会多返回
/// `liked` / `collected` / `purchased` / `commented` 这些标记，
/// 所以有 token 就带上，没有就当匿名请求（token 过期也不报错，直接降级为匿名）。
// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn api_url(path: &str, query: &[(String, String)]) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(&format!("{BASE}{path}")).map_err(|e| e.to_string())?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            if !v.is_empty() {
                pairs.append_pair(k, v);
            }
        }
    }
    Ok(url)
}

/// 解析统一返回结构：`code != 200` 视为失败（`msg` 为原因）；HTTP 401 = 未登录/过期。
async fn read_json(state: &AppState, resp: reqwest::Response, request: Option<&RequestAuth>) -> Result<Value, String> {
    let status = resp.status();
    let code_status = status.as_u16();
    let text = resp.text().await.map_err(|e| format!("读取拾木空间响应失败: {e}"))?;
    if code_status == 401 {
        if let Some(request) = request { clear_session(state, Some(request))?; }
        return Err(ERR_EXPIRED.to_string());
    }
    if let Some(request) = request {
        state.shimu.lock().unwrap().check_request(request)?;
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "拾木空间返回了非 JSON 响应（HTTP {code_status}）：{}",
            text.chars().take(120).collect::<String>()
        )
    })?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(200);
    if code != 200 {
        return Err(v
            .get("msg")
            .and_then(|m| m.as_str())
            .filter(|m| !m.is_empty())
            .map(|m| m.to_string())
            .unwrap_or_else(|| format!("拾木空间请求失败（code {code}）")));
    }
    Ok(v)
}

async fn get_authed(state: &AppState, path: &str, query: Vec<(String, String)>) -> Result<Value, String> {
    let request = current_token(state)?;
    get_with_token(state, path, query, &request.token, Some(&request)).await
}

async fn get_with_token(state: &AppState, path: &str, query: Vec<(String, String)>, token: &str, request: Option<&RequestAuth>) -> Result<Value, String> {
    if let Some(request) = request { state.shimu.lock().unwrap().check_request(request)?; }
    let url = api_url(path, &query)?;
    let resp = state
        .client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("请求拾木空间失败: {e}"))?;
    read_json(state, resp, request).await
}

/// 匿名可访问的接口：有登录态就带上（多拿个性化标记），没有就裸请求。
async fn get_public(state: &AppState, path: &str, query: Vec<(String, String)>) -> Result<Value, String> {
    let url = api_url(path, &query)?;
    let mut req = state.client.get(url);
    let request = current_token(state).ok();
    if let Some(auth) = request.as_ref() {
        req = req.bearer_auth(&auth.token);
    }
    let resp = req.send().await.map_err(|e| format!("请求拾木空间失败: {e}"))?;
    read_json(state, resp, request.as_ref()).await
}

async fn post_authed(state: &AppState, path: &str) -> Result<Value, String> {
    let request = current_token(state)?;
    post_for_request(state, path, &request).await
}

async fn post_for_request(state: &AppState, path: &str, request: &RequestAuth) -> Result<Value, String> {
    state.shimu.lock().unwrap().check_request(request)?;
    let url = api_url(path, &[])?;
    let resp = state
        .client
        .post(url)
        .bearer_auth(&request.token)
        .send()
        .await
        .map_err(|e| format!("请求拾木空间失败: {e}"))?;
    read_json(state, resp, Some(request)).await
}

// ---------------------------------------------------------------------------
// 命令：配置 / 登录
// ---------------------------------------------------------------------------

/// 当前接入配置（设置页与登录窗口展示用）
///
/// `devMode` 为 true 时才允许在界面上填写 client_id / client_secret：
/// 仅开发构建允许测试接入。
#[tauri::command]
pub fn shimu_config(app_state: tauri::State<'_, AppState>) -> Value {
    let dev = development_credentials(&app_state);
    let broker = crate::oauth_broker::shimu_broker().is_some();
    let client_id = if cfg!(debug_assertions) { app_state.settings().shimu_client_id.clone() } else { String::new() };
    json!({
        "host": HOST,
        "base": BASE,
        "authorizeUrl": format!("{HOST}/oauth/authorize"),
        "redirectUri": REDIRECT_URI,
        "scope": SCOPE,
        "clientId": client_id,
        "configured": broker || dev.is_some(),
        "secretConfigured": dev.is_some(),
        "devMode": cfg!(debug_assertions),
        "downloadDir": app_state.root.join("shimu").to_string_lossy(),
    })
}

/// 写入 client_id / client_secret（未提供 secret 时保持原值）
///
/// ⚠️ **仅开发构建可用**：正式包里这个入口不存在，凭证应由打包时注入。
#[tauri::command]
pub fn shimu_set_credentials(
    app_state: tauri::State<'_, AppState>,
    client_id: String,
    client_secret: Option<String>,
) -> Result<Value, String> {
    if !cfg!(debug_assertions) {
        return Err("正式构建通过 QookiX 服务授权".into());
    }
    let mut patch = serde_json::json!({ "shimu_client_id": client_id });
    if let Some(secret) = client_secret.filter(|value| !value.trim().is_empty()) {
        patch["shimu_client_secret"] = serde_json::Value::String(secret);
    }
    crate::settings::update_settings(&app_state, patch)?;
    Ok(shimu_config(app_state))
}

/// 第一步：生成授权页地址（前端用系统浏览器打开）
#[tauri::command]
pub async fn shimu_oauth_start(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let broker = crate::oauth_broker::shimu_broker();
    let dev = development_credentials(&app_state);
    if broker.is_none() && dev.is_none() {
        return Err("拾木空间授权服务尚未配置".into());
    }
    let state = uuid::Uuid::new_v4().simple().to_string();
    let verifier = broker.as_ref().map(|_| crate::oauth_broker::verifier());
    {
        let mut auth = app_state.shimu.lock().unwrap();
        let mut next = auth.clone();
        next.pending_state = Some((state.clone(), crate::util::now_secs() + STATE_TTL));
        next.broker_verifier = verifier.clone();
        next.generation = uuid::Uuid::new_v4();
        save_auth(&app_state, &next)?;
        *auth = next;
    }
    let url = match (broker, verifier, dev) {
        (Some(base), Some(verifier), _) => crate::oauth_broker::start(&app_state, &base, &state, &verifier).await?,
        (_, _, Some((client_id, _))) => format!(
            "{HOST}/oauth/authorize?client_id={}&redirect_uri={}&scope={}&state={state}",
            utf8_percent_encode(&client_id, NON_ALPHANUMERIC),
            utf8_percent_encode(REDIRECT_URI, NON_ALPHANUMERIC),
            utf8_percent_encode(SCOPE, NON_ALPHANUMERIC),
        ),
        _ => return Err("拾木空间授权服务尚未配置".into()),
    };
    app_state.shimu.lock().unwrap().check_callback(&state)?;
    Ok(json!({ "url": url, "state": state, "redirectUri": REDIRECT_URI }))
}

/// 第三步：用授权码换 `access_token`，并拉取用户资料
#[tauri::command]
pub async fn shimu_oauth_complete(
    app_state: tauri::State<'_, AppState>,
    code: String,
    state: String,
) -> Result<Value, String> {
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err("授权码为空".into());
    }
    // state 必须与发起时一致（防 CSRF / 回调来源校验）
    let verifier = {
        let auth = app_state.shimu.lock().unwrap();
        auth.check_callback(&state)?;
        auth.broker_verifier.clone()
    };
    let v = if let Some(verifier) = verifier {
        let broker = crate::oauth_broker::shimu_broker().ok_or("拾木空间授权服务尚未配置")?;
        crate::oauth_broker::complete(&app_state, &broker, &state, &code, &verifier).await?
    } else {
        let (client_id, client_secret) = development_credentials(&app_state)
            .ok_or("此授权需要重新发起，请通过浏览器授权")?;
        let response = crate::settings::oauth_http_client(&app_state).post(format!("{BASE}/api/oauth/token"))
            .form(&[
                ("code", code.as_str()), ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()), ("redirect_uri", REDIRECT_URI),
            ]).send().await.map_err(|err| format!("换取访问令牌失败: {err}"))?;
        let status = response.status();
        let bytes = crate::util::read_response_limited(response, 128 * 1024).await?;
        let data: Value = serde_json::from_slice(&bytes).map_err(|_| "访问令牌响应格式无效")?;
        if !status.is_success() || data.get("code").and_then(Value::as_i64).unwrap_or(200) != 200 {
            return Err(data.get("msg").and_then(Value::as_str).unwrap_or("换取访问令牌失败").to_owned());
        }
        data
    };
    let token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .ok_or("响应缺少 access_token")?
        .to_string();
    let expires_in = v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(DEFAULT_EXPIRES_IN);
    let expires_at = crate::util::now_secs().saturating_add(expires_in);
    app_state.shimu.lock().unwrap().check_callback(&state)?;
    // 使用刚兑换的令牌查询资料，保留服务端返回的精简资料作为网络失败时的回退。
    let user = match fetch_user_info(&app_state, &token, None).await {
        Ok(user) => Some(user),
        Err(err) if err == ERR_EXPIRED => return Err(err),
        Err(_) => v.get("user").cloned().filter(|u| !u.is_null()),
    };
    let mut auth = app_state.shimu.lock().unwrap();
    auth.check_callback(&state)?;
    let next = ShimuAuth {
        generation: uuid::Uuid::new_v4(),
        pending_state: None,
        broker_verifier: None,
        access_token: Some(token),
        expires_at,
        user: user.clone(),
    };
    save_auth(&app_state, &next)?;
    *auth = next;
    Ok(json!({ "user": user, "expiresAt": expires_at, "expiresIn": expires_in }))
}

/// 当前登录状态（`user` 为 null 表示未登录/已过期）
#[tauri::command]
pub fn shimu_status(app_state: tauri::State<'_, AppState>) -> Value {
    let auth = app_state.shimu.lock().unwrap();
    let has_token = auth.access_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false);
    let expired = has_token && auth.expires_at > 0 && crate::util::now_secs() >= auth.expires_at;
    json!({
        "loggedIn": has_token && !expired,
        "expired": expired,
        "expiresAt": auth.expires_at,
        "user": if expired { Value::Null } else { auth.user.clone().unwrap_or(Value::Null) },
    })
}

async fn fetch_user_info(state: &AppState, token: &str, request: Option<&RequestAuth>) -> Result<Value, String> {
    let v = get_with_token(state, "/getInfo", Vec::new(), token, request).await?;
    Ok(v.get("user").cloned().filter(|u| !u.is_null()).unwrap_or(v))
}

/// 刷新用户资料（余额/积分/会员会变）
#[tauri::command]
pub async fn shimu_user_info(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let request = current_token(&app_state)?;
    let user = fetch_user_info(&app_state, &request.token, Some(&request)).await?;
    {
        let mut auth = app_state.shimu.lock().unwrap();
        auth.check_request(&request)?;
        let mut next = auth.clone();
        next.user = Some(user.clone());
        save_auth(&app_state, &next)?;
        *auth = next;
    }
    Ok(user)
}

#[tauri::command]
pub fn shimu_logout(app_state: tauri::State<'_, AppState>) -> Result<(), String> {
    clear_session(&app_state, None)
}

// ---------------------------------------------------------------------------
// 命令：皮肤 / 披风上传（站点接口 `/api/user/skin`）
// ---------------------------------------------------------------------------

/// 上传一张 PNG 皮肤到拾木空间，替换当前登录用户正在穿戴的皮肤。
///
/// 站点接口：`POST /shimu/api/user/skin`，`multipart/form-data`，文件字段名
/// `skinfile`，鉴权用 OAuth 的 Bearer token（与其它开放 API 同一个 token）。
/// 站点**不接收** model 参数：经典/纤细由服务端按贴图自行判定，所以这里只传文件。
///
/// `skin` 接受 `data:image/png;base64,...` 或裸 base64（皮肤页的预览统一是 data URL）。
#[tauri::command]
pub async fn shimu_upload_skin(
    app_state: tauri::State<'_, AppState>,
    skin: String,
) -> Result<Value, String> {
    let bytes = crate::image_files::decode_png(&skin)?;
    let request = current_token(&app_state)?;
    let url = api_url("/api/user/skin", &[])?;
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("skin.png")
        .mime_str("image/png")
        .map_err(|e| e.to_string())?;
    // Content-Type（含 boundary）由 reqwest 自己生成，不能手动设置
    let form = reqwest::multipart::Form::new().part("skinfile", part);
    let resp = app_state
        .client
        .post(url)
        .bearer_auth(&request.token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("上传皮肤失败: {e}"))?;
    let v = read_json(&app_state, resp, Some(&request)).await?;
    // 站点把新皮肤地址放在 `data.skin`（顶层 `skin` 也兼容一下）
    let path = v
        .pointer("/data/skin")
        .or_else(|| v.get("skin"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    Ok(json!({ "skin": path }))
}

// ---------------------------------------------------------------------------
// 命令：开放 API
// ---------------------------------------------------------------------------

/// 帖子列表筛选条件（全部可选）
#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct ShimuPostQuery {
    pub category_id: Option<i64>,
    /// 多个板块（分类）合并查询：站点的 `categoryId` 是单值，需要跨板块时用它，
    /// 由后端逐板块取一页再合并（见 [`merge_category_posts`]）。
    /// 投影工坊默认用「投影 + 红石 + 生电 + 建筑」四个板块。
    pub category_ids: Option<Vec<i64>>,
    pub version_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub user_id: Option<i64>,
    pub title: Option<String>,
    pub keyword: Option<String>,
    /// all | content | author
    pub search_type: Option<String>,
    pub is_featured: Option<String>,
    pub is_top: Option<String>,
    /// hot | new | featured
    pub sort: Option<String>,
    /// 1 = 只返回带投影文件的帖子；0 = 只返回不带投影文件的；不传 = 不限制
    pub has_projection: Option<i32>,
    pub page_num: Option<u32>,
    pub page_size: Option<u32>,
}

impl ShimuPostQuery {
    /// 需要查询的板块 ID：`categoryIds` 优先（投影工坊的「投影相关」），
    /// 否则退化为单个 `categoryId`；丢弃非法值并去重排序。
    fn category_set(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = match &self.category_ids {
            Some(list) if !list.is_empty() => list.clone(),
            _ => self.category_id.into_iter().collect(),
        };
        ids.retain(|v| *v > 0);
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn to_query(&self) -> Vec<(String, String)> {
        let mut q: Vec<(String, String)> = Vec::new();
        let mut num = |k: &str, v: Option<i64>| {
            if let Some(v) = v {
                q.push((k.to_string(), v.to_string()));
            }
        };
        num("categoryId", self.category_id);
        num("versionId", self.version_id);
        num("tagId", self.tag_id);
        num("userId", self.user_id);
        num("hasProjection", self.has_projection.map(i64::from));
        for (k, v) in [
            ("title", &self.title),
            ("keyword", &self.keyword),
            ("searchType", &self.search_type),
            ("isFeatured", &self.is_featured),
            ("isTop", &self.is_top),
            ("sort", &self.sort),
        ] {
            if let Some(v) = v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                q.push((k.to_string(), v.to_string()));
            }
        }
        q.push(("pageNum".into(), self.page_num.unwrap_or(1).max(1).to_string()));
        q.push((
            "pageSize".into(),
            self.page_size.unwrap_or(20).clamp(1, 60).to_string(),
        ));
        q
    }
}

/// 帖子列表（`{ code, msg, total, rows }`）——匿名可访问。
///
/// 传 `categoryIds`（多个板块）时走合并路径；单个板块/不传时直接单次请求。
#[tauri::command]
pub async fn shimu_list_posts(
    app_state: tauri::State<'_, AppState>,
    query: Option<ShimuPostQuery>,
) -> Result<Value, String> {
    let q = query.unwrap_or_default();
    let ids = q.category_set();
    if ids.len() > 1 {
        return merge_category_posts(&app_state, &q, &ids).await;
    }
    let mut single = q;
    single.category_id = ids.first().copied();
    single.category_ids = None;
    get_public(&app_state, "/api/forum/post/list", single.to_query()).await
}

/// 板块（分类）列表：返回 `[{ categoryId, name, postCount, ... }]`——匿名可访问。
///
/// 投影工坊用它渲染板块筛选；站点 `categoryId` 只支持单值，所以「投影相关」
/// 需要把命中的板块 ID 一起回传，由 [`merge_category_posts`] 合并。
#[tauri::command]
pub async fn shimu_categories(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let v = get_public(&app_state, "/api/forum/category/list", Vec::new()).await?;
    Ok(v.get("data").cloned().unwrap_or_else(|| Value::Array(Vec::new())))
}

/// 合并多个板块的帖子列表。
///
/// 与内容中心「全部来源」同一策略：每页向每个板块各取 `pageSize` 条，
/// 合并去重后按所选维度统一排序，再截断到 `pageSize`；`total` 取各板块
/// 总数的最大值（可翻阅页数按最深的板块算）。
/// 单个板块失败不影响整体（全部失败才报错）。
async fn merge_category_posts(
    state: &AppState,
    q: &ShimuPostQuery,
    ids: &[i64],
) -> Result<Value, String> {
    let size = q.page_size.unwrap_or(20).clamp(1, 60) as usize;
    let reqs = ids.iter().map(|id| {
        let mut one = q.clone();
        one.category_id = Some(*id);
        one.category_ids = None;
        get_public(state, "/api/forum/post/list", one.to_query())
    });
    let results = futures_util::future::join_all(reqs).await;

    let mut rows: Vec<Value> = Vec::new();
    let mut total: u64 = 0;
    let mut ok = 0usize;
    let mut first_err: Option<String> = None;
    for res in results {
        match res {
            Ok(v) => {
                ok += 1;
                // 取各板块总数的最大值（而非求和）：合并后每页只截取 `pageSize` 条，
                // 最深板块的页数就是实际可翻阅的页数，求和会多出一截空页。
                total = total.max(v.get("total").and_then(|t| t.as_u64()).unwrap_or(0));
                if let Some(arr) = v.get("rows").and_then(|r| r.as_array()) {
                    rows.extend(arr.iter().cloned());
                }
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    if ok == 0 {
        return Err(first_err.unwrap_or_else(|| "请求拾木空间失败".into()));
    }
    // 同一帖子理论上只属于一个板块，去重只为兜底（跨版重复发布）
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| {
        r.get("postId")
            .and_then(|p| p.as_i64())
            .map(|id| seen.insert(id))
            .unwrap_or(true)
    });
    sort_merged_rows(&mut rows, q.sort.as_deref());
    rows.truncate(size);
    Ok(json!({ "code": 200, "msg": "操作成功", "total": total, "rows": rows }))
}

/// 合并后的统一排序：各板块单独排出来的顺序无法跨板块比较，只能按字段重排。
fn sort_merged_rows(rows: &mut [Value], sort: Option<&str>) {
    use std::cmp::Reverse;
    match sort.unwrap_or("new") {
        "hot" => rows.sort_by_key(|r| {
            Reverse(r.get("viewCount").and_then(|v| v.as_i64()).unwrap_or(0))
        }),
        "featured" => rows.sort_by(|a, b| {
            let fa = is_featured(a);
            let fb = is_featured(b);
            fb.cmp(&fa)
                .then_with(|| create_time(b).cmp(create_time(a)))
        }),
        // new（默认）
        _ => rows.sort_by(|a, b| create_time(b).cmp(create_time(a))),
    }
}

fn create_time(v: &Value) -> &str {
    v.get("createTime").and_then(|t| t.as_str()).unwrap_or("")
}

fn is_featured(v: &Value) -> bool {
    v.get("isFeatured").and_then(|f| f.as_str()).unwrap_or("0") == "1"
}

/// 帖子详情（付费内容未购买时 `content` 为空）——匿名可访问
#[tauri::command]
pub async fn shimu_post_detail(
    app_state: tauri::State<'_, AppState>,
    post_id: i64,
) -> Result<Value, String> {
    if post_id <= 0 {
        return Err("帖子 ID 无效".into());
    }
    get_public(&app_state, &format!("/api/forum/post/{post_id}"), Vec::new()).await
}

/// 热门榜——匿名可访问
#[tauri::command]
pub async fn shimu_hot_posts(
    app_state: tauri::State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Value, String> {
    let limit = limit.unwrap_or(10).clamp(1, 50);
    get_public(&app_state, "/api/forum/post/hot", vec![("limit".into(), limit.to_string())]).await
}

/// 精选——匿名可访问
#[tauri::command]
pub async fn shimu_featured_posts(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    get_public(&app_state, "/api/forum/post/featured", Vec::new()).await
}

/// 我的收藏
#[tauri::command]
pub async fn shimu_my_collects(
    app_state: tauri::State<'_, AppState>,
    page_num: Option<u32>,
    page_size: Option<u32>,
) -> Result<Value, String> {
    get_authed(
        &app_state,
        "/api/forum/post/my/collects",
        vec![
            ("pageNum".into(), page_num.unwrap_or(1).max(1).to_string()),
            ("pageSize".into(), page_size.unwrap_or(20).clamp(1, 60).to_string()),
        ],
    )
    .await
}

/// 权益校验：是否已购买该帖子 / 资源。
/// 文档：未登录不报错，直接返回 `entitled=false` → 同样走匿名可访问路径。
#[tauri::command]
pub async fn shimu_check_entitlement(
    app_state: tauri::State<'_, AppState>,
    biz_type: String,
    biz_id: i64,
) -> Result<Value, String> {
    let biz_type = normalize_biz_type(&biz_type)?;
    get_public(
        &app_state,
        "/api/order/check",
        vec![
            ("bizType".into(), biz_type),
            ("bizId".into(), biz_id.to_string()),
        ],
    )
    .await
}

/// 付费下载（购买资源，自动扣余额/积分并给作者分成）
#[tauri::command]
pub async fn shimu_purchase_download(
    app_state: tauri::State<'_, AppState>,
    file_id: i64,
) -> Result<Value, String> {
    if file_id <= 0 {
        return Err("资源 ID 无效".into());
    }
    post_authed(&app_state, &format!("/api/order/download/{file_id}")).await
}

/// 我的订单
#[tauri::command]
pub async fn shimu_my_orders(
    app_state: tauri::State<'_, AppState>,
    biz_type: Option<String>,
    page_num: Option<u32>,
    page_size: Option<u32>,
) -> Result<Value, String> {
    let mut q: Vec<(String, String)> = Vec::new();
    if let Some(t) = biz_type.as_deref().filter(|t| !t.trim().is_empty()) {
        q.push(("bizType".into(), normalize_biz_type(t)?));
    }
    q.push(("pageNum".into(), page_num.unwrap_or(1).max(1).to_string()));
    q.push(("pageSize".into(), page_size.unwrap_or(20).clamp(1, 60).to_string()));
    get_authed(&app_state, "/api/order/my", q).await
}

fn normalize_biz_type(t: &str) -> Result<String, String> {
    let t = t.trim().to_ascii_uppercase();
    match t.as_str() {
        "POST" | "DOWNLOAD" | "RECHARGE_BALANCE" | "RECHARGE_MEMBER" => Ok(t),
        _ => Err(format!("未知的业务类型：{t}")),
    }
}

// ---------------------------------------------------------------------------
// 命令：下载附件（投影文件）
// ---------------------------------------------------------------------------

/// 只允许 http(s) 且指向公网主机：防止把任意 URL（含内网/localhost）当作下载源。
fn allowed_download_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "下载地址无效".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("只支持 http(s) 下载地址".into());
    }
    let host = url.host_str().ok_or("下载地址缺少主机名")?.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".local") || !host.contains('.') {
        return Err("下载地址指向本机/内网，已拒绝".into());
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let internal = match ip {
            std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        };
        if internal {
            return Err("下载地址指向内网，已拒绝".into());
        }
    }
    Ok(url)
}

/// 从 Content-Disposition 里取文件名（`filename*=UTF-8''x` 优先于 `filename="x"`）
fn filename_from_disposition(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    if let Some(pos) = lower.find("filename*=") {
        let rest = &value[pos + "filename*=".len()..];
        let rest = rest.trim_start_matches('"');
        // 形如 UTF-8''%E6%8A%95%E5%BD%B1.litematic
        let encoded = rest.split("''").nth(1).unwrap_or(rest);
        let end = encoded.find(';').unwrap_or(encoded.len());
        let decoded = percent_encoding::percent_decode_str(&encoded[..end])
            .decode_utf8_lossy()
            .to_string();
        if !decoded.trim().is_empty() {
            return Some(decoded.trim().to_string());
        }
    }
    let pos = lower.find("filename=")?;
    let rest = value[pos + "filename=".len()..].trim_start_matches('"');
    let end = rest.find(['"', ';']).unwrap_or(rest.len());
    let name = rest[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 下载帖子附件。`file_id` 存在时先校验权益，未购买则自动下单（与站内下载等价）。
///
/// 返回 `{ path, name, size }`，`path` 可直接交给 `schematic_preview_open`。
#[tauri::command]
pub async fn shimu_download_file(
    app_state: tauri::State<'_, AppState>,
    url: String,
    file_name: Option<String>,
    file_id: Option<i64>,
) -> Result<Value, String> {
    let url = allowed_download_url(&url)?;
    let request = current_token(&app_state)?;

    // 付费资源：先查权益，没买就下单（免费/已购时接口会返回对应提示，可忽略）
    if let Some(fid) = file_id.filter(|f| *f > 0) {
        let entitled = get_with_token(
            &app_state,
            "/api/order/check",
            vec![("bizType".into(), "DOWNLOAD".into()), ("bizId".into(), fid.to_string())],
            &request.token,
            Some(&request),
        )
        .await
        .ok()
        .and_then(|v| v.get("entitled").and_then(|e| e.as_bool()))
        .unwrap_or(false);
        if !entitled {
            if let Err(e) = post_for_request(&app_state, &format!("/api/order/download/{fid}"), &request).await {
                // 「免费资源 / 已购买」不算失败，其余（余额不足等）必须报给用户
                let ignorable = e.contains("免费") || e.contains("已购买");
                if !ignorable {
                    return Err(e);
                }
            }
        }
    }

    app_state.shimu.lock().unwrap().check_request(&request)?;
    let resp = app_state
        .client
        .get(url.clone())
        .bearer_auth(&request.token)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 {
            clear_session(&app_state, Some(&request))?;
            return Err(ERR_EXPIRED.to_string());
        }
        return Err(format!("下载失败（HTTP {}）", status.as_u16()));
    }
    if resp.content_length().unwrap_or(0) > MAX_DOWNLOAD {
        return Err("附件超过 512 MiB，未下载".into());
    }
    let header_name = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(filename_from_disposition);
    let bytes = resp.bytes().await.map_err(|e| format!("读取附件失败: {e}"))?;
    app_state.shimu.lock().unwrap().check_request(&request)?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err("附件超过 512 MiB，未保存".into());
    }

    let name = file_name
        .or(header_name)
        .or_else(|| {
            // URL 末段看起来像文件名（带扩展名）时才用
            url.path_segments()
                .and_then(|s| s.last())
                .filter(|s| s.contains('.') && !s.contains('/'))
                .map(str::to_string)
        })
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "shimu-download".to_string());
    let name = if crate::fsutil::is_safe_filename(&name) {
        name
    } else {
        // 名字不合法（带路径分隔符等）时退化成按扩展名生成的安全名字
        let ext = std::path::Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        format!("shimu-download.{ext}")
    };
    // 投影预览按扩展名判格式，没扩展名时补一个（拾木空间的帖子基本都是投影）
    let name = if std::path::Path::new(&name).extension().is_none() {
        format!("{name}.litematic")
    } else {
        name
    };

    let dir = app_state.root.join("shimu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建下载目录失败: {e}"))?;
    let path = dir.join(&name);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| format!("写入附件失败: {e}"))?;

    Ok(json!({
        "path": path.to_string_lossy(),
        "name": name,
        "size": bytes.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_url_blocks_local_and_non_http() {
        assert!(allowed_download_url("https://www.mcimg.com/shimu/file/1.litematic").is_ok());
        assert!(allowed_download_url("http://cdn.example.com/a.litematic").is_ok());
        assert!(allowed_download_url("file:///C:/windows/notepad.exe").is_err());
        assert!(allowed_download_url("ftp://example.com/a").is_err());
        assert!(allowed_download_url("http://localhost/a").is_err());
        assert!(allowed_download_url("http://127.0.0.1/a").is_err());
        assert!(allowed_download_url("http://192.168.1.10/a").is_err());
        assert!(allowed_download_url("http://server/a").is_err());
    }

    #[test]
    fn disposition_filename_variants() {
        assert_eq!(
            filename_from_disposition("attachment; filename=\"house.litematic\"").as_deref(),
            Some("house.litematic")
        );
        assert_eq!(
            filename_from_disposition("attachment; filename*=UTF-8''%E6%8A%95%E5%BD%B1.litematic")
                .as_deref(),
            Some("投影.litematic")
        );
        assert_eq!(filename_from_disposition("inline"), None);
    }

    #[test]
    fn post_query_builds_expected_pairs() {
        let q = ShimuPostQuery {
            keyword: Some("  中世纪  ".into()),
            sort: Some("hot".into()),
            category_id: Some(3),
            page_num: None,
            page_size: Some(999),
            ..Default::default()
        }
        .to_query();
        assert!(q.contains(&("keyword".to_string(), "中世纪".to_string())));
        assert!(q.contains(&("sort".to_string(), "hot".to_string())));
        assert!(q.contains(&("categoryId".to_string(), "3".to_string())));
        assert!(q.contains(&("pageNum".to_string(), "1".to_string())));
        // pageSize 被夹到上限
        assert!(q.contains(&("pageSize".to_string(), "60".to_string())));
    }

    #[test]
    fn category_set_merges_and_dedupes() {
        // categoryIds 优先，非法值丢弃、去重排序
        let merged = ShimuPostQuery {
            category_ids: Some(vec![5, 1, 5, 0, -2, 3, 4]),
            ..Default::default()
        };
        assert_eq!(merged.category_set(), vec![1, 3, 4, 5]);
        // 只有单个 categoryId 时也能识别
        let single = ShimuPostQuery {
            category_id: Some(3),
            ..Default::default()
        };
        assert_eq!(single.category_set(), vec![3]);
        // categoryIds 为空数组时退回 categoryId
        let fallback = ShimuPostQuery {
            category_id: Some(1),
            category_ids: Some(vec![]),
            ..Default::default()
        };
        assert_eq!(fallback.category_set(), vec![1]);
        assert!(ShimuPostQuery::default().category_set().is_empty());
        // 多板块不会进单次查询（走合并路径）
        assert!(!merged.to_query().iter().any(|(k, _)| k == "categoryIds"));
    }

    #[test]
    fn merged_rows_sort_by_selected_dimension() {
        let mk = |id: i64, time: &str, views: i64, featured: &str| {
            json!({
                "postId": id,
                "createTime": time,
                "viewCount": views,
                "isFeatured": featured,
            })
        };
        let base = vec![
            mk(1, "2026-01-01 00:00:00", 5, "1"),
            mk(2, "2026-09-25 10:33:11", 156, "0"),
            mk(3, "2026-05-05 08:00:00", 900, "0"),
        ];
        let ids = |rows: &[Value]| {
            rows.iter()
                .map(|r| r.get("postId").and_then(|v| v.as_i64()).unwrap_or(0))
                .collect::<Vec<_>>()
        };

        let mut newest = base.clone();
        sort_merged_rows(&mut newest, Some("new"));
        assert_eq!(ids(&newest), vec![2, 3, 1]);

        let mut hot = base.clone();
        sort_merged_rows(&mut hot, Some("hot"));
        assert_eq!(ids(&hot), vec![3, 2, 1]);

        let mut featured = base.clone();
        sort_merged_rows(&mut featured, Some("featured"));
        assert_eq!(ids(&featured), vec![1, 2, 3]);
    }

    #[test]
    fn biz_type_is_normalized_and_validated() {
        assert_eq!(normalize_biz_type("download").unwrap(), "DOWNLOAD");
        assert_eq!(normalize_biz_type(" POST ").unwrap(), "POST");
        assert!(normalize_biz_type("nope").is_err());
    }
}
