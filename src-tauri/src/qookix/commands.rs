//! QookiX 账号与好友命令（薄命令层，参数搬运 + 会话落盘）。

use super::api::{self, ApiError};
use super::{clear_session, save_session, status};
use crate::state::AppState;
use serde_json::{json, Value};

#[tauri::command]
pub fn qookix_status(state: tauri::State<'_, AppState>) -> Value {
    super::status(&state)
}

/// 登录（用户名或邮箱 + 密码）。成功后令牌写入加密文件。
#[tauri::command]
pub async fn qookix_login(
    state: tauri::State<'_, AppState>,
    login: String,
    password: String,
) -> Result<Value, ApiError> {
    let login = login.trim().to_string();
    if login.is_empty() || password.is_empty() {
        return Err(ApiError::new(400, "invalid_request", "请输入账号和密码"));
    }
    let data = api::post_public(
        &state,
        "/api/v1/auth/login",
        json!({ "login": login, "password": password }),
    )
    .await?;
    let tokens = data
        .get("tokens")
        .ok_or_else(|| ApiError::internal("登录响应缺少 tokens"))?;
    api::apply_tokens(&state, tokens)?;
    if let Some(user) = data.get("user") {
        let mut s = state.qookix();
        s.user = Some(user.clone());
    }
    save_session(&state).map_err(ApiError::internal)?;
    Ok(status(&state))
}

/// 注册（公开接口）。
///
/// 服务端开启邮箱验证时只返回 `email_verification_required` 并发送验证邮件，
/// **不下发令牌**；关闭验证时直接给 tokens，这里顺手把会话写进去。
#[tauri::command]
pub async fn qookix_register(
    state: tauri::State<'_, AppState>,
    username: String,
    email: String,
    password: String,
    nickname: Option<String>,
) -> Result<Value, ApiError> {
    let username = username.trim().to_string();
    let email = email.trim().to_string();
    if username.is_empty() || email.is_empty() || password.is_empty() {
        return Err(ApiError::new(
            400,
            "invalid_request",
            "用户名、邮箱和密码都要填",
        ));
    }
    let nickname = nickname.unwrap_or_default().trim().to_string();
    let data = api::post_public(
        &state,
        "/api/v1/auth/register",
        json!({
            "username": username,
            "email": email,
            "password": password,
            "nickname": nickname,
        }),
    )
    .await?;
    if let Some(tokens) = data.get("tokens") {
        api::apply_tokens(&state, tokens)?;
        if let Some(user) = data.get("user") {
            let mut s = state.qookix();
            s.user = Some(user.clone());
        }
        save_session(&state).map_err(ApiError::internal)?;
    }
    Ok(data)
}

/// 重发验证邮件（公开）：无论账号是否存在都返回同一句话，避免枚举邮箱
#[tauri::command]
pub async fn qookix_resend_verification(
    state: tauri::State<'_, AppState>,
    login: String,
) -> Result<Value, ApiError> {
    api::post_public(
        &state,
        "/api/v1/auth/resend-verification",
        json!({ "login": login.trim() }),
    )
    .await
}

/// 忘记密码：发重置邮件（同样不暴露账号是否存在）。
/// 邮件里的链接指向网页版重置页，令牌只在那个页面里用。
#[tauri::command]
pub async fn qookix_forgot_password(
    state: tauri::State<'_, AppState>,
    login: String,
) -> Result<Value, ApiError> {
    api::post_public(
        &state,
        "/api/v1/auth/forgot-password",
        json!({ "login": login.trim() }),
    )
    .await
}

#[tauri::command]
pub async fn qookix_logout(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    // 服务端登出失败也要清本地：本地令牌本来就已经作废或不可用
    let _ = api::post(&state, "/api/v1/auth/logout", json!({})).await;
    clear_session(&state);
    Ok(status(&state))
}

/// 当前用户 + 统计（同时刷新本地缓存的用户资料）
#[tauri::command]
pub async fn qookix_me(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    let data = api::get(&state, "/api/v1/me", Vec::new()).await?;
    if let Some(user) = data.get("user") {
        {
            let mut s = state.qookix();
            s.user = Some(user.clone());
        }
        let _ = save_session(&state);
    }
    Ok(data)
}

/// 从本地账号里取 Minecraft access token。
///
/// **令牌不出这个模块**：只在 Rust 内部传给绑定接口，不经过 IPC、不进日志。
fn mc_token_of(account: &crate::models::Account) -> Result<String, ApiError> {
    match account {
        crate::models::Account::Microsoft { msa_access_token, .. } => {
            // 注意：这个字段名带 msa_ 前缀，装的其实是 Minecraft access token
            // （见 accounts.rs::xbox_login）。它的有效期本地没有记录，
            // 所以不要拿 msa_expires_at 判断，直接请求、401 再刷新。
            if msa_access_token.trim().is_empty() {
                Err(ApiError::new(
                    400,
                    "mc_token_malformed",
                    "本地没有 Minecraft 登录令牌，请重新登录游戏账号",
                ))
            } else {
                Ok(msa_access_token.clone())
            }
        }
        _ => Err(ApiError::new(
            400,
            "mc_offline_account",
            "当前账号不是正版登录，请先在启动器登录正版 Minecraft 账号",
        )),
    }
}

fn find_account(state: &AppState, uuid: &str) -> Result<crate::models::Account, ApiError> {
    let accounts = crate::accounts::load_accounts(state);
    let hit = if uuid.is_empty() {
        accounts.into_iter().find(|a| matches!(a, crate::models::Account::Microsoft { .. }))
    } else {
        accounts.into_iter().find(|a| a.uuid() == uuid)
    };
    hit.ok_or_else(|| ApiError::new(404, "mc_no_account", "没有找到这个游戏账号"))
}

fn remember_user(state: &AppState, data: &Value) {
    if let Some(user) = data.get("user") {
        {
            let mut s = state.qookix();
            s.user = Some(user.clone());
        }
        let _ = save_session(state);
    }
}

async fn bind_with_token(state: &AppState, token: &str) -> Result<Value, ApiError> {
    api::post(
        state,
        "/api/v1/me/mc/bind",
        json!({ "mc_access_token": token }),
    )
    .await
}

/// 绑定正版 Minecraft 账号（也用于「刷新正版名」）。
///
/// 服务端拿 token 去调 `/minecraft/profile` 验证归属，一次请求完成。
/// `mc_token_invalid` 时自动刷新一次账号令牌再重试，用户无感。
#[tauri::command]
pub async fn qookix_bind_mc(
    state: tauri::State<'_, AppState>,
    uuid: Option<String>,
) -> Result<Value, ApiError> {
    let uuid = uuid.unwrap_or_default();
    let account = find_account(&state, uuid.trim())?;
    let token = mc_token_of(&account)?;
    match bind_with_token(&state, &token).await {
        Ok(data) => {
            remember_user(&state, &data);
            Ok(data)
        }
        Err(e) if e.code == "mc_token_invalid" => {
            // 令牌多半过期了：刷新一次（会重新走 Xbox 登录并落库）再试
            let fresh = crate::accounts::refresh_microsoft(&state, &account)
                .await
                .map_err(|msg| {
                    ApiError::new(
                        401,
                        "mc_token_invalid",
                        format!("游戏账号令牌已失效，请重新登录：{msg}"),
                    )
                })?;
            let retry = mc_token_of(&fresh)?;
            let data = bind_with_token(&state, &retry).await?;
            remember_user(&state, &data);
            Ok(data)
        }
        Err(e) => Err(e),
    }
}

/// 发起更换邮箱：验证链接会发到新邮箱（旧邮箱收到提醒）
#[tauri::command]
pub async fn qookix_change_email(
    state: tauri::State<'_, AppState>,
    new_email: String,
) -> Result<Value, ApiError> {
    let email = new_email.trim().to_lowercase();
    if email.is_empty() {
        return Err(ApiError::new(400, "invalid_email", "请填写新邮箱"));
    }
    api::post(&state, "/api/v1/me/email/start", json!({ "new_email": email })).await
}

/// 解绑正版账号（24 小时内不能重新绑定，服务端强制）
#[tauri::command]
pub async fn qookix_unbind_mc(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    let data = api::post(&state, "/api/v1/me/mc/unbind", json!({ "confirm": true })).await?;
    remember_user(&state, &data);
    Ok(data)
}

/// 批量取好友绑定的 Minecraft 账号（画皮肤头用）
#[tauri::command]
pub async fn qookix_user_profiles(
    state: tauri::State<'_, AppState>,
    uids: Vec<i64>,
) -> Result<Value, ApiError> {
    let mut ids: Vec<String> = uids
        .into_iter()
        .filter(|u| *u > 0)
        .take(100)
        .map(|u| u.to_string())
        .collect();
    ids.dedup();
    if ids.is_empty() {
        return Ok(json!({ "profiles": [] }));
    }
    api::get(
        &state,
        "/api/v1/users/profiles",
        vec![("uids".to_string(), ids.join(","))],
    )
    .await
}

/// 修改昵称
#[tauri::command]
pub async fn qookix_update_nickname(
    state: tauri::State<'_, AppState>,
    nickname: String,
) -> Result<Value, ApiError> {
    let nickname = nickname.trim().to_string();
    if nickname.is_empty() || nickname.chars().count() > 24 {
        return Err(ApiError::new(400, "invalid_nickname", "昵称需为 1-24 个字符"));
    }
    let data = api::patch(&state, "/api/v1/me", json!({ "nickname": nickname })).await?;
    if let Some(user) = data.get("user") {
        {
            let mut s = state.qookix();
            s.user = Some(user.clone());
        }
        let _ = save_session(&state);
    }
    Ok(data)
}

#[tauri::command]
pub async fn qookix_search_users(
    state: tauri::State<'_, AppState>,
    q: String,
    limit: Option<u32>,
) -> Result<Value, ApiError> {
    let q = q.trim().to_string();
    if q.is_empty() {
        return Ok(json!({ "users": [] }));
    }
    if q.chars().count() > 32 {
        return Err(ApiError::new(400, "invalid_query", "搜索词最多 32 个字符"));
    }
    api::get(
        &state,
        "/api/v1/users/search",
        vec![
            ("q".to_string(), q),
            ("limit".to_string(), limit.unwrap_or(10).clamp(1, 20).to_string()),
        ],
    )
    .await
}

#[tauri::command]
pub async fn qookix_friends(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::get(&state, "/api/v1/friends", Vec::new()).await
}

/// `kind` = incoming / outgoing
#[tauri::command]
pub async fn qookix_requests(
    state: tauri::State<'_, AppState>,
    kind: Option<String>,
) -> Result<Value, ApiError> {
    let kind = match kind.as_deref() {
        Some("outgoing") => "outgoing",
        _ => "incoming",
    };
    api::get(
        &state,
        "/api/v1/friends/requests",
        vec![("type".to_string(), kind.to_string())],
    )
    .await
}

#[tauri::command]
pub async fn qookix_send_friend_request(
    state: tauri::State<'_, AppState>,
    to_uid: Option<i64>,
    to_username: Option<String>,
    message: Option<String>,
) -> Result<Value, ApiError> {
    let username = to_username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if to_uid.is_none() && username.is_none() {
        return Err(ApiError::new(400, "invalid_request", "缺少目标用户"));
    }
    let message = message
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(100).collect::<String>());
    let mut body = json!({});
    if let Some(uid) = to_uid {
        body["to_uid"] = json!(uid);
    }
    if let Some(name) = username {
        body["to_username"] = json!(name);
    }
    if let Some(msg) = message {
        body["message"] = json!(msg);
    }
    api::post(&state, "/api/v1/friends/requests", body).await
}

/// `action` = accept / reject / cancel
#[tauri::command]
pub async fn qookix_respond_request(
    state: tauri::State<'_, AppState>,
    request_id: i64,
    action: String,
) -> Result<Value, ApiError> {
    let path = match action.as_str() {
        "accept" => "/api/v1/friends/requests/accept",
        "reject" => "/api/v1/friends/requests/reject",
        "cancel" => "/api/v1/friends/requests/cancel",
        other => {
            return Err(ApiError::new(
                400,
                "invalid_action",
                format!("未知的申请操作: {other}"),
            ))
        }
    };
    api::post(&state, path, json!({ "request_id": request_id })).await
}

#[tauri::command]
pub async fn qookix_remove_friend(
    state: tauri::State<'_, AppState>,
    uid: i64,
) -> Result<Value, ApiError> {
    api::post(&state, "/api/v1/friends/remove", json!({ "uid": uid })).await
}

#[tauri::command]
pub async fn qookix_blocks(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::get(&state, "/api/v1/blocks", Vec::new()).await
}

/// 拉黑 / 解除拉黑
#[tauri::command]
pub async fn qookix_set_block(
    state: tauri::State<'_, AppState>,
    uid: i64,
    blocked: bool,
) -> Result<Value, ApiError> {
    if blocked {
        api::post(&state, "/api/v1/blocks", json!({ "uid": uid })).await
    } else {
        api::delete(
            &state,
            "/api/v1/blocks",
            vec![("uid".to_string(), uid.to_string())],
        )
        .await
    }
}
