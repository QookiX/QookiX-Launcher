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
mod poller;
mod rooms;
mod sync;

pub use commands::*;
pub use poller::{cleanup_on_exit, start_background};
pub use rooms::*;
pub use sync::*;

use crate::state::AppState;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 未登录：前端据此弹登录框，而不是当成错误提示
pub const CODE_NOT_LOGGED_IN: &str = "not_logged_in";

/// 正在分享到目录的房间（内存态，不落盘）
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
#[derive(Default)]
pub struct QookixSession {
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
        access_token,
        refresh_token: read("refresh_token"),
        expires_at: v.get("expires_at").and_then(|e| e.as_u64()).unwrap_or(0),
        user: v.get("user").cloned().filter(|u| !u.is_null()),
        cursor: 0,
        share: None,
    }
}

/// 落盘登录态（两个令牌分别加密）。未登录时删除文件。
pub fn save_session(state: &AppState) -> Result<(), String> {
    let (access, refresh, expires_at, user) = {
        let s = state.qookix();
        (
            s.access_token.clone(),
            s.refresh_token.clone(),
            s.expires_at,
            s.user.clone().unwrap_or(Value::Null),
        )
    };
    if access.is_empty() {
        let _ = std::fs::remove_file(auth_path(state));
        return Ok(());
    }
    let payload = json!({
        "access_token": crate::secret::protect(&access)?,
        "refresh_token": crate::secret::protect(&refresh)?,
        "expires_at": expires_at,
        "user": user,
    });
    let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    std::fs::write(auth_path(state), text).map_err(|e| format!("保存 QookiX 登录状态失败: {e}"))
}

/// 清除登录态：内存 + 磁盘 + 正在分享的房间
pub fn clear_session(state: &AppState) {
    {
        let mut s = state.qookix();
        if let Some(share) = s.share.take() {
            share.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        *s = QookixSession::default();
    }
    let _ = std::fs::remove_file(auth_path(state));
}

/// 前端可直接消费的登录态
pub fn status(state: &AppState) -> Value {
    let s = state.qookix();
    json!({
        "loggedIn": s.logged_in(),
        "expiresAt": s.expires_at,
        "user": s.user.clone().unwrap_or(Value::Null),
        "share": s.share.as_ref().map(|r| json!({
            "roomId": r.room_id,
            "code": r.code,
            "title": r.title,
            "visibility": r.visibility,
            // 有没有附带环境清单（前端据此显示「已附带 N 个 mod」）
            "hasPack": !r.pack.is_empty(),
            "modsCount": r.mods_count,
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
