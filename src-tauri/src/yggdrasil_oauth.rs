//! LittleSkin「通过 OAuth 访问 Yggdrasil API」：授权码流登录皮肤站账号。
//!
//! 传统 Yggdrasil 登录要启动器直接处理用户名/密码，无法支持二步验证与统一
//! 授权体验。LittleSkin 据此开放了 OAuth2：启动器走授权码流拿到 OAuth 访问
//! 令牌，再调两个新 API 把它换成可直接用于 Yggdrasil API 的 Minecraft 令牌，
//! 最终登记成一个普通的皮肤站（Yggdrasil）启动账号。
//!
//! 流程（与官方文档一致，见 manual.littleskin.cn/feature/oauth-for-yggdrasil）：
//! 1. `yggdrasil_oauth_start` 生成带 `state` 的授权页地址，前端用系统浏览器打开；
//! 2. 用户在浏览器里点「允许」，站点 302 回 `qookix://oauth/littleskin/callback?code=..&state=..`；
//! 3. deep-link 插件把回调交给 JS，JS 调 `yggdrasil_oauth_complete(code, state)`：
//!    后端用授权码换 OAuth access_token，再拉用户名下全部角色，把 token 临时记在
//!    内存里，返回角色列表供前端挑选（单角色时前端直接进入下一步）；
//! 4. 前端选定角色后调 `yggdrasil_oauth_add_account(profileId)`：后端用 OAuth
//!    access_token 调 `/api/yggdrasil/authserver/oauth` 拿到 Minecraft 令牌
//!    （响应格式与 Yggdrasil 登录 API 完全一致），登记为皮肤站账号，清掉临时 token。
//!
//! ⚠️ 桌面启动器没有「我方服务端」，`client_secret` 只能编译期注入或开发期填写，
//! 不能被认为真的保密；与拾木空间同，secret 泄露的最坏后果是他人可代换 token。
//!
//! 仅在配置了 client_id（编译期 `LITTLESKIN_CLIENT_ID` 或设置页 dev 模式填写）
//! 时才在界面露出 OAuth2 入口——「有专门设置的才有」。

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

/// 编译期注入的 client_id / client_secret
pub const BUILTIN_CLIENT_ID: &str = match option_env!("LITTLESKIN_CLIENT_ID") {
    Some(v) => v,
    None => "",
};
pub const BUILTIN_CLIENT_SECRET: &str = match option_env!("LITTLESKIN_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

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
    pub profiles: Vec<YggOauthProfile>,
    pub server: String,
    pub server_name: String,
    pub links: YggOauthLinks,
}

/// 进行中的 OAuth 流程状态：待校验的 state（落盘，扛冷启动回调）+ 换到的 OAuth
/// access_token（仅内存，选角色前临时持有）。
#[derive(Default)]
pub struct YggOauthPending {
    pub pending_state: Option<(String, u64)>,
    pub oauth_token: Option<String>,
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
        oauth_token: None,
    }
}

fn save_pending(state: &AppState, pending: &YggOauthPending) -> Result<(), String> {
    if pending.pending_state.is_none() {
        let _ = std::fs::remove_file(pending_path(state));
        return Ok(());
    }
    let payload = json!({
        "pending_state": pending.pending_state.as_ref().map(|(s, _)| s.clone()),
        "pending_expires": pending.pending_state.as_ref().map(|(_, e)| *e),
    });
    std::fs::write(pending_path(state), serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| format!("保存 LittleSkin 授权状态失败: {e}"))
}

pub fn effective_client_id(state: &AppState) -> String {
    let user = state.settings().littleskin_client_id.trim().to_string();
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
        .littleskin_client_secret
        .clone()
        .unwrap_or_default();
    let user = user.trim().to_string();
    if !user.is_empty() {
        return user;
    }
    BUILTIN_CLIENT_SECRET.trim().to_string()
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
    let client_id = effective_client_id(&app_state);
    json!({
        "host": HOST,
        "apiRoot": API_ROOT,
        "authorizeUrl": format!("{HOST}/oauth/authorize"),
        "redirectUri": REDIRECT_URI,
        "scope": SCOPE,
        "clientId": client_id,
        "configured": !client_id.is_empty(),
        "secretConfigured": !effective_client_secret(&app_state).is_empty(),
        "devMode": cfg!(debug_assertions),
    })
}

/// 写入 client_id / client_secret（未提供 secret 时保持原值）
///
/// ⚠️ **仅开发构建可用**：正式包里凭证应由打包时注入。
#[tauri::command]
pub fn yggdrasil_set_credentials(
    app_state: tauri::State<'_, AppState>,
    client_id: String,
    client_secret: Option<String>,
) -> Result<Value, String> {
    if !cfg!(debug_assertions) {
        return Err(
            "正式构建不允许在客户端设置接入凭证，请在打包时注入 \
             LITTLESKIN_CLIENT_ID / LITTLESKIN_CLIENT_SECRET"
                .into(),
        );
    }
    {
        let mut s = app_state.settings_mut();
        s.littleskin_client_id = client_id.trim().to_string();
        if let Some(sec) = client_secret {
            let sec = sec.trim().to_string();
            if !sec.is_empty() {
                s.littleskin_client_secret = Some(sec);
            }
        }
    }
    crate::settings::persist(&app_state)?;
    Ok(yggdrasil_oauth_config(app_state))
}

/// 第一步：生成授权页地址（前端用系统浏览器打开）
#[tauri::command]
pub fn yggdrasil_oauth_start(app_state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let client_id = effective_client_id(&app_state);
    if client_id.is_empty() {
        return Err(if cfg!(debug_assertions) {
            "尚未配置 LittleSkin client_id：请在设置里填写，或用 LITTLESKIN_CLIENT_ID 重新构建".to_string()
        } else {
            "当前版本未内置 LittleSkin 接入凭证，无法完成 OAuth 登录".to_string()
        });
    }
    let st = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut pending = app_state.ygg_oauth.lock().unwrap();
        pending.pending_state = Some((st.clone(), now_secs() + STATE_TTL));
        pending.oauth_token = None;
        save_pending(&app_state, &pending)?;
    }
    let url = format!(
        "{HOST}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        utf8_percent_encode(&client_id, NON_ALPHANUMERIC),
        utf8_percent_encode(REDIRECT_URI, NON_ALPHANUMERIC),
        utf8_percent_encode(SCOPE, NON_ALPHANUMERIC),
        st
    );
    Ok(json!({ "url": url, "state": st, "redirectUri": REDIRECT_URI }))
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
        let ok = matches!(&pending.pending_state, Some((s, exp)) if s == &state && now_secs() <= *exp);
        if !ok {
            return Err("授权校验失败（state 不匹配或已过期），请重新发起授权".into());
        }
    }
    let client_id = effective_client_id(&app_state);
    let client_secret = effective_client_secret(&app_state);
    if client_id.is_empty() {
        return Err("尚未配置 LittleSkin client_id".into());
    }
    if client_secret.is_empty() {
        return Err("尚未配置 LittleSkin client_secret（换 token 必需，只在后端使用）".into());
    }

    // ---- 兑换 OAuth access_token ----
    let resp = app_state
        .client
        .post(format!("{HOST}/oauth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
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
    let v: Value = serde_json::from_str(&text).map_err(|_| {
        format!("token 接口返回非 JSON（HTTP {status}）：{}", text.chars().take(120).collect::<String>())
    })?;
    if status != 200 {
        return Err(oauth_error(&v, &format!("换取 access_token 失败（HTTP {status}）")));
    }
    let oauth_token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .ok_or("token 接口未返回 access_token")?
        .to_string();

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
        pending.pending_state = None;
        pending.oauth_token = Some(oauth_token);
        save_pending(&app_state, &pending)?;
    }

    Ok(YggOauthCompleteResult {
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
) -> Result<Account, String> {
    let profile_id = profile_id.trim().to_string();
    if profile_id.is_empty() {
        return Err("缺少角色信息".into());
    }
    let oauth_token = {
        let mut pending = app_state.ygg_oauth.lock().unwrap();
        let token = pending
            .oauth_token
            .clone()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "授权会话已失效，请重新发起 OAuth 登录".to_string())?;
        // 先清掉，失败也不留半截会话
        pending.oauth_token = None;
        token
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

    let account = Account::Yggdrasil {
        uuid: pid,
        username: pname,
        created: now_secs(),
        access_token,
        client_token,
        server: API_ROOT.to_string(),
        server_name: "LittleSkin".to_string(),
    };
    let mut list = crate::accounts::load_accounts(app_state.inner());
    list.retain(|a| a.uuid() != account.uuid());
    list.push(account.clone());
    crate::accounts::save_accounts(app_state.inner(), &list)?;
    Ok(account)
}
