//! QookiX 账号与好友命令（薄命令层，参数搬运 + 会话落盘）。

use super::api::{self, ApiError};
use super::{edit_session, status, QookixSession, SessionStatus};
use crate::state::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

fn begin_auth(state: &AppState) -> Uuid {
    let attempt = Uuid::new_v4();
    let mut session = state.qookix();
    session.auth_attempt = attempt;
    session.challenge = None;
    attempt
}

fn finish_auth(state: &AppState, attempt: Uuid, tokens: &Value, user: Option<&Value>) -> Result<(), ApiError> {
    edit_session(state, |session| {
        if session.auth_attempt != attempt {
            return Err(ApiError::session_changed());
        }
        let mut next = QookixSession { generation: Uuid::new_v4(), user: user.cloned(), ..Default::default() };
        api::apply_tokens(&mut next, tokens)?;
        *session = next;
        Ok(())
    })
}

#[tauri::command]
pub fn qookix_status(state: tauri::State<'_, AppState>) -> SessionStatus {
    super::status(&state)
}

#[derive(Serialize)]
pub struct LoginChallenge {
    two_factor_required: bool,
    challenge_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    methods: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum LoginReply {
    Challenge(LoginChallenge),
    Session(SessionStatus),
}

#[derive(Serialize)]
pub struct RegistrationReply {
    email_verification_required: bool,
    email_sent: Option<bool>,
    message: Option<String>,
}

/// 登录（用户名或邮箱 + 密码）。成功后令牌写入加密文件。
#[tauri::command]
pub async fn qookix_login(
    state: tauri::State<'_, AppState>,
    login: String,
    password: String,
) -> Result<LoginReply, ApiError> {
    let login = login.trim().to_string();
    if login.is_empty() || password.is_empty() {
        return Err(ApiError::new(400, "invalid_request", "请输入账号和密码"));
    }
    let attempt = begin_auth(&state);
    let data = api::post_public(
        &state,
        "/api/v1/auth/login",
        json!({ "login": login, "password": password }),
    )
    .await?;
    // 账号开了两步验证：密码正确但没发令牌，把 challenge 原样交给前端弹验证码框
    if data.get("two_factor_required").and_then(|v| v.as_bool()) == Some(true) {
        let challenge_token = data.get("challenge_token").and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::internal("二步验证响应缺少 challenge_token"))?;
        let mut session = state.qookix();
        if session.auth_attempt != attempt {
            return Err(ApiError::session_changed());
        }
        session.challenge = Some(challenge_token.to_owned());
        return Ok(LoginReply::Challenge(LoginChallenge {
            two_factor_required: true,
            challenge_token: challenge_token.into(),
            expires_in: data.get("expires_in").and_then(Value::as_u64),
            methods: data.get("methods").cloned().and_then(|value| serde_json::from_value(value).ok()),
        }));
    }
    let tokens = data
        .get("tokens")
        .ok_or_else(|| ApiError::internal("登录响应缺少 tokens"))?;
    finish_auth(&state, attempt, tokens, data.get("user"))?;
    Ok(LoginReply::Session(status(&state)))
}

/// 两步验证第二步：用 challenge_token + 动态码（或恢复码）换正式令牌。
#[tauri::command]
pub async fn qookix_login_2fa(
    state: tauri::State<'_, AppState>,
    challenge_token: String,
    code: String,
) -> Result<SessionStatus, ApiError> {
    let challenge_token = challenge_token.trim().to_string();
    let code = code.trim().to_string();
    if challenge_token.is_empty() || code.is_empty() {
        return Err(ApiError::new(400, "invalid_request", "请输入验证码"));
    }
    let attempt = {
        let session = state.qookix();
        if session.challenge.as_deref() != Some(challenge_token.as_str()) {
            return Err(ApiError::session_changed());
        }
        session.auth_attempt
    };
    let data = api::post_public(
        &state,
        "/api/v1/auth/login/2fa",
        json!({ "challenge_token": challenge_token, "code": code }),
    )
    .await?;
    let tokens = data
        .get("tokens")
        .ok_or_else(|| ApiError::internal("二步验证响应缺少 tokens"))?;
    finish_auth(&state, attempt, tokens, data.get("user"))?;
    Ok(status(&state))
}

/// 两步验证状态
#[tauri::command]
pub async fn qookix_2fa_status(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::get(&state, "/api/v1/me/2fa", Vec::new()).await
}

/// 两步验证第 1 步：生成种子 + 恢复码（恢复码只返回这一次）
#[tauri::command]
pub async fn qookix_2fa_setup(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::post(&state, "/api/v1/me/2fa/setup", json!({})).await
}

/// 两步验证第 2 步：输入一次动态码完成开启
#[tauri::command]
pub async fn qookix_2fa_enable(
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<Value, ApiError> {
    api::post(
        &state,
        "/api/v1/me/2fa/enable",
        json!({ "code": code.trim() }),
    )
    .await
}

/// 关闭两步验证：密码 + 动态码/恢复码
#[tauri::command]
pub async fn qookix_2fa_disable(
    state: tauri::State<'_, AppState>,
    password: String,
    code: String,
) -> Result<Value, ApiError> {
    if password.is_empty() || code.trim().is_empty() {
        return Err(ApiError::new(400, "invalid_request", "请输入密码和验证码"));
    }
    api::post(
        &state,
        "/api/v1/me/2fa/disable",
        json!({ "password": password, "code": code.trim() }),
    )
    .await
}

/// 修改密码：成功后服务端会吊销旧令牌并返回新令牌，这里直接换掉本地会话。
#[tauri::command]
pub async fn qookix_change_password(
    state: tauri::State<'_, AppState>,
    old_password: String,
    new_password: String,
) -> Result<(), ApiError> {
    if new_password.len() < 8 || new_password.len() > 72 {
        return Err(ApiError::new(
            400,
            "invalid_password",
            "密码长度需为 8-72 字节",
        ));
    }
    if old_password.is_empty() {
        return Err(ApiError::new(400, "invalid_old_password", "请输入原密码"));
    }
    let generation = api::generation(&state);
    let data = api::request_for_session(
        &state,
        generation,
        reqwest::Method::POST,
        "/api/v1/me/password",
        vec![],
        Some(json!({ "old_password": old_password, "new_password": new_password })),
    )
    .await?;
    if let Some(tokens) = data.get("tokens") {
        edit_session(&state, |session| {
            session.check_generation(generation)?;
            api::apply_tokens(session, tokens)
        })?;
    }
    Ok(())
}

/// 注册策略（公开）：是否要邀请码 / 人机验证 / 协议地址
#[tauri::command]
pub async fn qookix_register_policy(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::get_public(&state, "/api/v1/auth/register-policy", Vec::new()).await
}

/// 服务状态（公开）：在线、降级、今日翻译量等对外字段
#[tauri::command]
pub async fn qookix_service_status(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::get_public(&state, "/public/status", Vec::new()).await
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
    invite_code: Option<String>,
    agreed_terms: bool,
) -> Result<RegistrationReply, ApiError> {
    let username = username.trim().to_string();
    let email = email.trim().to_string();
    if username.is_empty() || email.is_empty() || password.is_empty() {
        return Err(ApiError::new(
            400,
            "invalid_request",
            "用户名、邮箱和密码都要填",
        ));
    }
    if !agreed_terms {
        return Err(ApiError::new(
            400,
            "terms_not_accepted",
            "请先阅读并同意用户协议",
        ));
    }
    let nickname = nickname.unwrap_or_default().trim().to_string();
    // 邀请制时服务端强制校验；空串不传，避免把 "" 当成一个无效邀请码
    let invite_code = invite_code
        .unwrap_or_default()
        .trim()
        .to_uppercase()
        .replace([' ', '-'], "");
    let mut body = json!({
        "username": username,
        "email": email,
        "password": password,
        "nickname": nickname,
        "agreed_terms": true,
    });
    if !invite_code.is_empty() {
        body["invite_code"] = json!(invite_code);
    }
    let attempt = begin_auth(&state);
    let data = api::post_public(&state, "/api/v1/auth/register", body).await?;
    if let Some(tokens) = data.get("tokens") {
        finish_auth(&state, attempt, tokens, data.get("user"))?;
    }
    Ok(RegistrationReply {
        email_verification_required: data.get("email_verification_required").and_then(Value::as_bool).unwrap_or(false),
        email_sent: data.get("email_sent").and_then(Value::as_bool),
        message: data.get("message").and_then(Value::as_str).map(str::to_owned),
    })
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
pub async fn qookix_logout(state: tauri::State<'_, AppState>) -> Result<SessionStatus, ApiError> {
    api::logout(&state).await?;
    Ok(status(&state))
}

/// 当前用户 + 统计（同时刷新本地缓存的用户资料）
#[tauri::command]
pub async fn qookix_me(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    let generation = api::generation(&state);
    let data = api::request_for_session(&state, generation, reqwest::Method::GET, "/api/v1/me", vec![], None).await?;
    remember_user(&state, generation, &data)?;
    Ok(data)
}

/// 从本地账号里取 Minecraft access token。
///
/// **令牌不出这个模块**：只在 Rust 内部传给绑定接口，不经过 IPC、不进日志。
fn mc_token_of(account: &crate::models::Account) -> Result<String, ApiError> {
    match account {
        crate::models::Account::Microsoft {
            msa_access_token, ..
        } => {
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
        accounts
            .into_iter()
            .find(|a| matches!(a, crate::models::Account::Microsoft { .. }))
    } else {
        accounts.into_iter().find(|a| a.uuid() == uuid)
    };
    hit.ok_or_else(|| ApiError::new(404, "mc_no_account", "没有找到这个游戏账号"))
}

fn remember_user(state: &AppState, generation: Uuid, data: &Value) -> Result<(), ApiError> {
    if let Some(user) = data.get("user") {
        edit_session(state, |session| {
            session.check_generation(generation)?;
            session.user = Some(user.clone());
            Ok(())
        })?;
    }
    Ok(())
}

async fn bind_with_token(state: &AppState, generation: Uuid, token: &str) -> Result<Value, ApiError> {
    api::request_for_session(
        state,
        generation,
        reqwest::Method::POST,
        "/api/v1/me/mc/bind",
        vec![],
        Some(json!({ "mc_access_token": token })),
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
    let generation = api::generation(&state);
    match bind_with_token(&state, generation, &token).await {
        Ok(data) => {
            remember_user(&state, generation, &data)?;
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
            let data = bind_with_token(&state, generation, &retry).await?;
            remember_user(&state, generation, &data)?;
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
    api::post(
        &state,
        "/api/v1/me/email/start",
        json!({ "new_email": email }),
    )
    .await
}

/// 解绑正版账号（24 小时内不能重新绑定，服务端强制）
#[tauri::command]
pub async fn qookix_unbind_mc(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    let generation = api::generation(&state);
    let data = api::request_for_session(&state, generation, reqwest::Method::POST, "/api/v1/me/mc/unbind", vec![], Some(json!({ "confirm": true }))).await?;
    remember_user(&state, generation, &data)?;
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
        return Err(ApiError::new(
            400,
            "invalid_nickname",
            "昵称需为 1-24 个字符",
        ));
    }
    let generation = api::generation(&state);
    let data = api::request_for_session(&state, generation, reqwest::Method::PATCH, "/api/v1/me", vec![], Some(json!({ "nickname": nickname }))).await?;
    remember_user(&state, generation, &data)?;
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
            (
                "limit".to_string(),
                limit.unwrap_or(10).clamp(1, 20).to_string(),
            ),
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
