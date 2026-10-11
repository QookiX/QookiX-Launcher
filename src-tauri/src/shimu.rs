//! 拾木空间（www.mcimg.com）第三方应用接入：OAuth 授权登录 + 开放 API。
//!
//! ⚠️ 合规边界：拾木空间账号**不是启动账号**。它的用户信息里带 `mcUuid`，
//! 但那只作资料展示/识别用 —— 绝不能拿它去充当启动身份或绕过 Minecraft
//! 正版验证（Minecraft 使用准则禁止规避正版验证）。启动游戏仍走启动器
//! 既有的微软 / 离线 / 皮肤站账号体系。
//!
//! 流程（与官方《第三方应用接入文档》一致）：
//! 1. `shimu_oauth_start` 生成带 `state` 的授权页地址，前端用系统浏览器打开；
//! 2. 用户在浏览器里点「允许」，站点 302 回 `qookix://oauth/callback?code=..&state=..`；
//! 3. deep-link 插件把回调交给 JS，JS 调 `shimu_oauth_complete(code, state)`；
//! 4. 后端（**只有后端能拿 client_secret**）用授权码换 `access_token`，
//!    再拉 `getInfo` 补齐资料，落盘（Windows 走 DPAPI 加密）。
//!
//! 注意：`access_token` 有效期 3 天且**不提供 refresh_token**，过期后必须让
//! 用户重新走一遍授权流程（前端据 `shimu_status` 的 `expired` 提示）。
//!
//! ⚠️ 桌面启动器没有「我方服务端」，`client_secret` 只能编译期注入或让用户
//! 自行填写；它**不能**被认为真的保密。拾木空间接口本身以用户 token 为准，
//! secret 泄露的最坏后果是他人可代换 token，故优先用环境变量注入。

use crate::state::AppState;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// 编译期注入的 client_id / client_secret（`SHIMU_CLIENT_ID` / `SHIMU_CLIENT_SECRET`）
pub const BUILTIN_CLIENT_ID: &str = match option_env!("SHIMU_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
pub const BUILTIN_CLIENT_SECRET: &str = match option_env!("SHIMU_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 登录态
// ---------------------------------------------------------------------------

/// 拾木空间登录态：一次性授权码流程的 `state` + 当前 token + 用户资料。
#[derive(Default)]
pub struct ShimuAuth {
    /// 待回调校验的 (state, 过期时间戳)
    pub pending_state: Option<(String, u64)>,
    pub access_token: Option<String>,
    /// token 过期时间戳（0 = 未知，按不限期处理）
    pub expires_at: u64,
    /// `getInfo` 返回的用户对象（原样保存，字段随服务端升级自动跟上）
    pub user: Option<Value>,
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
        let _ = std::fs::remove_file(auth_path(state));
        return Ok(());
    }
    let payload = json!({
        "access_token": match token {
            Some(t) => crate::secret::protect(t)?,
            None => String::new(),
        },
        "expires_at": auth.expires_at,
        "pending_state": auth.pending_state.as_ref().map(|(s, _)| s.clone()),
        "pending_expires": auth.pending_state.as_ref().map(|(_, e)| *e),
        "user": auth.user.clone().unwrap_or(Value::Null),
    });
    std::fs::write(
        auth_path(state),
        serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("保存拾木空间登录状态失败: {e}"))
}

fn clear_session(state: &AppState) {
    let mut auth = state.shimu.lock().unwrap();
    auth.access_token = None;
    auth.expires_at = 0;
    auth.pending_state = None;
    auth.user = None;
    let _ = std::fs::remove_file(auth_path(state));
}

pub fn effective_client_id(state: &AppState) -> String {
    let user = state.settings.read().unwrap().shimu_client_id.trim().to_string();
    if !user.is_empty() {
        return user;
    }
    BUILTIN_CLIENT_ID.trim().to_string()
}

pub fn effective_client_secret(state: &AppState) -> String {
    let user = state
        .settings
        .read()
        .unwrap()
        .shimu_client_secret
        .clone()
        .unwrap_or_default();
    let user = user.trim().to_string();
    if !user.is_empty() {
        return user;
    }
    BUILTIN_CLIENT_SECRET.trim().to_string()
}

fn current_token(state: &AppState) -> Result<String, String> {
    let auth = state.shimu.lock().unwrap();
    let token = auth
        .access_token
        .as_deref()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ERR_NOT_LOGGED_IN.to_string())?;
    if auth.expires_at > 0 && now() >= auth.expires_at {
        return Err(ERR_EXPIRED.to_string());
    }
    Ok(token.to_string())
}

/// 可选登录：帖子列表/详情等接口**匿名就能看**，登录后才会多返回
/// `liked` / `collected` / `purchased` / `commented` 这些标记，
/// 所以有 token 就带上，没有就当匿名请求（token 过期也不报错，直接降级为匿名）。
fn optional_token(state: &AppState) -> Option<String> {
    let auth = state.shimu.lock().unwrap();
    let token = auth.access_token.as_deref().filter(|t| !t.is_empty())?;
    if auth.expires_at > 0 && now() >= auth.expires_at {
        return None;
    }
    Some(token.to_string())
}

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
async fn read_json(state: &AppState, resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let code_status = status.as_u16();
    let text = resp.text().await.map_err(|e| format!("读取拾木空间响应失败: {e}"))?;
    if code_status == 401 {
        clear_session(state);
        return Err(ERR_EXPIRED.to_string());
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
    let token = current_token(state)?;
    let url = api_url(path, &query)?;
    let resp = state
        .client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("请求拾木空间失败: {e}"))?;
    read_json(state, resp).await
}

/// 匿名可访问的接口：有登录态就带上（多拿个性化标记），没有就裸请求。
async fn get_public(state: &AppState, path: &str, query: Vec<(String, String)>) -> Result<Value, String> {
    let url = api_url(path, &query)?;
    let mut req = state.client.get(url);
    if let Some(token) = optional_token(state) {
        req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let resp = req.send().await.map_err(|e| format!("请求拾木空间失败: {e}"))?;
    read_json(state, resp).await
}

async fn post_authed(state: &AppState, path: &str) -> Result<Value, String> {
    let token = current_token(state)?;
    let url = api_url(path, &[])?;
    let resp = state
        .client
        .post(url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("请求拾木空间失败: {e}"))?;
    read_json(state, resp).await
}

// ---------------------------------------------------------------------------
// 命令：配置 / 登录
// ---------------------------------------------------------------------------

/// 当前接入配置（设置页与登录窗口展示用）
///
/// `devMode` 为 true 时才允许在界面上填写 client_id / client_secret：
/// 那是**开发/联调用**的入口，正式打包必须靠 SHIMU_CLIENT_ID /
/// SHIMU_CLIENT_SECRET 编译期注入，不能把凭证交给终端用户去填。
#[tauri::command]
pub fn shimu_config(app_state: tauri::State<'_, AppState>) -> Value {
    let client_id = effective_client_id(&app_state);
    json!({
        "host": HOST,
        "base": BASE,
        "authorizeUrl": format!("{HOST}/oauth/authorize"),
        "redirectUri": REDIRECT_URI,
        "scope": SCOPE,
        "clientId": client_id,
        "configured": !client_id.is_empty(),
        "secretConfigured": !effective_client_secret(&app_state).is_empty(),
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
        return Err(
            "正式构建不允许在客户端设置接入凭证，请在打包时注入 \
             SHIMU_CLIENT_ID / SHIMU_CLIENT_SECRET"
                .into(),
        );
    }
    {
        let mut s = app_state.settings.write().unwrap();
        s.shimu_client_id = client_id.trim().to_string();
        if let Some(sec) = client_secret {
            let sec = sec.trim().to_string();
            if !sec.is_empty() {
                s.shimu_client_secret = Some(sec);
            }
        }
    }
    crate::settings::persist(&app_state)?;
    Ok(shimu_config(app_state))
}

/// 第一步：生成授权页地址（前端用系统浏览器打开）
#[tauri::command]
pub fn shimu_oauth_start(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let client_id = effective_client_id(&app_state);
    if client_id.is_empty() {
        return Err(if cfg!(debug_assertions) {
            "尚未配置拾木空间 client_id：请在登录窗口的「接入设置」里填写，\
             或用 SHIMU_CLIENT_ID 重新构建"
                .to_string()
        } else {
            "当前版本未内置拾木空间接入凭证，无法完成登录".to_string()
        });
    }
    let st = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut auth = app_state.shimu.lock().unwrap();
        auth.pending_state = Some((st.clone(), now() + STATE_TTL));
        // 也落盘：用户在浏览器里登录期间重启启动器，回调仍能通过校验
        save_auth(&app_state, &auth)?;
    }
    let url = format!(
        "{HOST}/oauth/authorize?client_id={}&redirect_uri={}&scope={}&state={}",
        utf8_percent_encode(&client_id, NON_ALPHANUMERIC),
        utf8_percent_encode(REDIRECT_URI, NON_ALPHANUMERIC),
        utf8_percent_encode(SCOPE, NON_ALPHANUMERIC),
        st
    );
    Ok(json!({ "url": url, "state": st, "redirectUri": REDIRECT_URI }))
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
    {
        let auth = app_state.shimu.lock().unwrap();
        let ok = matches!(&auth.pending_state, Some((s, exp)) if s == &state && now() <= *exp);
        if !ok {
            return Err("授权校验失败（state 不匹配或已过期），请重新发起授权".into());
        }
    }
    let client_id = effective_client_id(&app_state);
    let client_secret = effective_client_secret(&app_state);
    if client_id.is_empty() {
        return Err("尚未配置拾木空间 client_id".into());
    }
    if client_secret.is_empty() {
        return Err("尚未配置拾木空间 client_secret（换 token 必需，只在后端使用）".into());
    }

    let resp = app_state
        .client
        .post(format!("{BASE}/api/oauth/token"))
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", REDIRECT_URI),
        ])
        .send()
        .await
        .map_err(|e| format!("换取 access_token 失败: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| format!("token 接口返回非 JSON（HTTP {status}）：{}", text.chars().take(120).collect::<String>()))?;
    let code_field = v.get("code").and_then(|c| c.as_i64()).unwrap_or(200);
    if code_field != 200 {
        return Err(v
            .get("msg")
            .and_then(|m| m.as_str())
            .unwrap_or("换取 access_token 失败")
            .to_string());
    }
    let token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .ok_or("响应缺少 access_token")?
        .to_string();
    let expires_in = v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(DEFAULT_EXPIRES_IN);

    {
        let mut auth = app_state.shimu.lock().unwrap();
        auth.pending_state = None;
        auth.access_token = Some(token);
        auth.expires_at = now() + expires_in;
        auth.user = v.get("user").cloned().filter(|u| !u.is_null());
        save_auth(&app_state, &auth)?;
    }
    // 用 getInfo 补齐余额/积分/会员/mcUuid/皮肤披风（token 里只有精简资料）
    Ok(match fetch_user_info(&app_state).await {
        Ok(user) => json!({ "user": user, "expiresAt": now() + expires_in, "expiresIn": expires_in }),
        Err(_) => {
            let auth = app_state.shimu.lock().unwrap();
            json!({
                "user": auth.user.clone().unwrap_or(Value::Null),
                "expiresAt": auth.expires_at,
                "expiresIn": expires_in,
            })
        }
    })
}

/// 当前登录状态（`user` 为 null 表示未登录/已过期）
#[tauri::command]
pub fn shimu_status(app_state: tauri::State<'_, AppState>) -> Value {
    let auth = app_state.shimu.lock().unwrap();
    let has_token = auth.access_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false);
    let expired = has_token && auth.expires_at > 0 && now() >= auth.expires_at;
    json!({
        "loggedIn": has_token && !expired,
        "expired": expired,
        "expiresAt": auth.expires_at,
        "user": if expired { Value::Null } else { auth.user.clone().unwrap_or(Value::Null) },
    })
}

async fn fetch_user_info(state: &AppState) -> Result<Value, String> {
    let v = get_authed(state, "/getInfo", Vec::new()).await?;
    Ok(v.get("user").cloned().filter(|u| !u.is_null()).unwrap_or(v))
}

/// 刷新用户资料（余额/积分/会员会变）
#[tauri::command]
pub async fn shimu_user_info(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let user = fetch_user_info(&app_state).await?;
    {
        let mut auth = app_state.shimu.lock().unwrap();
        auth.user = Some(user.clone());
        let _ = save_auth(&app_state, &auth);
    }
    Ok(user)
}

#[tauri::command]
pub fn shimu_logout(app_state: tauri::State<'_, AppState>) -> Result<(), String> {
    clear_session(&app_state);
    Ok(())
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
    let token = current_token(&app_state)?;

    // 付费资源：先查权益，没买就下单（免费/已购时接口会返回对应提示，可忽略）
    if let Some(fid) = file_id.filter(|f| *f > 0) {
        let entitled = get_authed(
            &app_state,
            "/api/order/check",
            vec![("bizType".into(), "DOWNLOAD".into()), ("bizId".into(), fid.to_string())],
        )
        .await
        .ok()
        .and_then(|v| v.get("entitled").and_then(|e| e.as_bool()))
        .unwrap_or(false);
        if !entitled {
            if let Err(e) = post_authed(&app_state, &format!("/api/order/download/{fid}")).await {
                // 「免费资源 / 已购买」不算失败，其余（余额不足等）必须报给用户
                let ignorable = e.contains("免费") || e.contains("已购买");
                if !ignorable {
                    return Err(e);
                }
            }
        }
    }

    let resp = app_state
        .client
        .get(url.clone())
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        if status.as_u16() == 401 {
            clear_session(&app_state);
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
    let name = if crate::util::is_safe_filename(&name) {
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
