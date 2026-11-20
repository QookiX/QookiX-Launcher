//! QookiX 账号：好友与陶瓦联机房间目录。
//!
//! 这是一个**可选**账号，只承载「好友 + 房间发现」两件事，不接管 Minecraft
//! 登录，不登录时启动器其余功能完全不受影响。
//!
//! 令牌落盘前经 `secret::protect`（Windows DPAPI）加密；refresh token 一次性
//! 使用，所以并发 401 必须共用一把刷新闸门（`AppState::qookix_refresh`），
//! 否则互相作废会把用户直接踢下线。

pub mod api;
mod commands;
mod skin;
mod poller;
mod rooms;
mod sync;

pub use commands::*;
pub use skin::*;
pub use poller::{cleanup_on_exit, start_background};
pub use rooms::*;
pub use sync::*;

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use uuid::Uuid;

/// 未登录：前端据此弹登录框，而不是当成错误提示
pub const CODE_NOT_LOGGED_IN: &str = "not_logged_in";

/// 正在分享到目录的房间（内存态，不落盘）
#[derive(Clone)]
pub struct RoomShare {
    pub room_id: String,
    pub code: String,
    pub title: String,
    pub visibility: String,
    /// 环境清单 JSON（房主共享环境时带上；换房重报时要一起带上，否则清单会丢）
    pub pack: String,
    pub mods_count: u32,
    pub cancel: Arc<AtomicBool>,
}

/// QookiX 登录态 + 运行时游标
#[derive(Clone, Default)]
pub struct QookixSession {
    pub generation: Uuid,
    pub auth_attempt: Uuid,
    pub challenge: Option<String>,
    pub access_token: String,
    pub refresh_token: String,
    /// access token 过期时间（Unix 秒），0 表示未知
    pub expires_at: u64,
    /// 服务端返回的用户对象（原样保存，字段随服务端升级自动跟上）
    pub user: Option<Value>,
    /// 通知增量游标（上一次响应的 server_time）
    pub cursor: u64,
    pub share: Option<RoomShare>,
}

impl QookixSession {
    pub fn logged_in(&self) -> bool {
        !self.access_token.is_empty()
    }

    pub fn check_generation(&self, expected: Uuid) -> Result<(), api::ApiError> {
        (self.generation == expected).then_some(()).ok_or_else(api::ApiError::session_changed)
    }
}

fn auth_path(state: &AppState) -> PathBuf {
    state.root.join("qookix_auth.json")
}

/// 启动时从磁盘恢复登录态；解密失败一律视为未登录。
pub fn load_saved(root: &Path) -> QookixSession {
    let Ok(text) = std::fs::read_to_string(root.join("qookix_auth.json")) else {
        return QookixSession::default();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return QookixSession::default();
    };
    let read = |key: &str| {
        v.get(key)
            .and_then(|t| t.as_str())
            .and_then(|t| crate::secret::unprotect(t).ok())
            .unwrap_or_default()
    };
    let access_token = read("access_token");
    // 只有 refresh token 没有 access token 时不可用，直接当未登录
    if access_token.is_empty() {
        return QookixSession::default();
    }
    QookixSession {
        generation: Uuid::new_v4(),
        access_token,
        refresh_token: read("refresh_token"),
        expires_at: v.get("expires_at").and_then(|e| e.as_u64()).unwrap_or(0),
        user: v.get("user").cloned().filter(|u| !u.is_null()),
        ..Default::default()
    }
}

/// 落盘登录态（两个令牌分别加密）。未登录时删除文件。
fn write_session(state: &AppState, session: &QookixSession) -> Result<(), String> {
    if !session.logged_in() {
        return match std::fs::remove_file(auth_path(state)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("清除 QookiX 登录状态失败: {err}")),
        };
    }
    let payload = json!({
        "access_token": crate::secret::protect(&session.access_token)?,
        "refresh_token": crate::secret::protect(&session.refresh_token)?,
        "expires_at": session.expires_at,
        "user": session.user,
    });
    let text = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;
    crate::fsutil::atomic_write(&auth_path(state), &text).map_err(|e| format!("保存 QookiX 登录状态失败: {e}"))
}

/// 所有登录态变更在同一临界区校验、落盘和提交。
fn edit_session<T>(state: &AppState, edit: impl FnOnce(&mut QookixSession) -> Result<T, api::ApiError>) -> Result<T, api::ApiError> {
    let mut current = state.qookix();
    let mut next = current.clone();
    let result = edit(&mut next)?;
    write_session(state, &next).map_err(api::ApiError::internal)?;
    if current.generation != next.generation {
        if let Some(share) = &current.share {
            share.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    *current = next;
    Ok(result)
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct AccountView {
    id: u64,
    username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    nickname_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nickname_review_reason: Option<String>,
    role: String,
    status: String,
    created_at: u64,
    email_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mc_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mc_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mc_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mc_bound_at: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareView {
    room_id: String,
    code: String,
    title: String,
    visibility: String,
    has_pack: bool,
    mods_count: u32,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    logged_in: bool,
    expires_at: u64,
    user: Option<AccountView>,
    pub(super) share: Option<ShareView>,
}

/// 只序列化前端所需的登录态字段。
pub fn status(state: &AppState) -> SessionStatus {
    let s = state.qookix();
    SessionStatus {
        logged_in: s.logged_in(),
        expires_at: s.expires_at,
        user: s.user.as_ref().and_then(|user| AccountView::deserialize(user).ok()),
        share: s.share.as_ref().map(|r| ShareView {
            room_id: r.room_id.clone(),
            code: r.code.clone(),
            title: r.title.clone(),
            visibility: r.visibility.clone(),
            has_pack: !r.pack.is_empty(),
            mods_count: r.mods_count,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_wire_example_exposes_only_account_view_fields() {
        let raw_user = json!({
            "id": 3,
            "username": "example",
            "nickname": "Example",
            "role": "user",
            "status": "active",
            "created_at": 100,
            "email_verified": true,
            "access_token": "example-only",
        });
        let session = SessionStatus {
            logged_in: true,
            expires_at: 200,
            user: Some(AccountView::deserialize(&raw_user).unwrap()),
            share: None,
        };
        let fixtures: Value = serde_json::from_str(include_str!("../../../src/api/wire-contracts.json")).unwrap();
        assert_eq!(serde_json::to_value(session).unwrap(), fixtures["session"]);
    }

    #[test]
    fn session_defaults_to_logged_out() {
        let s = QookixSession::default();
        assert!(!s.logged_in());
        assert!(s.user.is_none());
    }

    #[test]
    fn load_saved_tolerates_missing_and_broken_files() {
        let dir = std::env::temp_dir().join("qookix_test_auth_missing");
        let _ = std::fs::create_dir_all(&dir);
        assert!(!load_saved(&dir).logged_in());

        std::fs::write(dir.join("qookix_auth.json"), "{not json").unwrap();
        assert!(!load_saved(&dir).logged_in());

        std::fs::write(dir.join("qookix_auth.json"), r#"{"access_token":"","refresh_token":""}"#).unwrap();
        assert!(!load_saved(&dir).logged_in());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
