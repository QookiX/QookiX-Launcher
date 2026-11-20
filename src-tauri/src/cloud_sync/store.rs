//! 云同步本地状态：GitHub 凭据、仓库信息、世界映射与自动同步设置。
//! 存放于数据目录 `cloud_sync/state.json`，凭据由系统密钥保护。

use crate::state::AppState;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct WorldLink {
    /// 实例 id
    pub instance_id: String,
    /// 世界目录名
    pub world_dir: String,
    /// 云端世界 id（uuid）
    pub world_id: String,
    /// 游戏退出后自动上传
    pub auto_sync: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(default)]
pub struct CloudState {
    pub session_id: String,
    /// 受系统密钥保护的 access_token
    pub access_token: String,
    /// 受系统密钥保护的 refresh_token
    pub refresh_token: String,
    /// access_token 过期时间（unix 秒）
    pub expires_at: u64,
    /// GitHub 登录名
    pub account: String,
    /// 仓库名
    pub repo_name: String,
    /// 仓库唯一 id
    pub repository_id: String,
    /// 每个世界保留的快照数上限
    pub keep_per_world: u32,
    /// 世界映射（本地目录 ↔ 云端世界 id / 自动同步开关）
    pub worlds: Vec<WorldLink>,
}

impl CloudState {
    /// 是否已授权（拿到令牌）。仓库是否就绪须单独检查 `repo_name`，
    /// 否则刚授权完、仓库还没建时会误判为未连接。
    pub fn connected(&self) -> bool {
        !self.account.is_empty() && !self.access_token.is_empty()
    }

    pub fn check_session(&self, expected: &Self) -> Result<(), String> {
        let same = if !expected.session_id.is_empty() {
            self.session_id == expected.session_id
        } else {
            self.session_id.is_empty()
                && self.account == expected.account
                && self.access_token == expected.access_token
                && self.refresh_token == expected.refresh_token
        };
        same.then_some(()).ok_or_else(|| "GitHub 授权已变化，请重试".into())
    }
}

fn state_path(state: &AppState) -> std::path::PathBuf {
    state.root.join("cloud_sync").join("state.json")
}

/// 兼容曾使用可逆混淆的凭据。
pub fn deobfuscate(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s) else {
        return String::new();
    };
    String::from_utf8(
        bytes
            .into_iter()
            .enumerate()
            .map(|(i, b)| b ^ (0x5A ^ (i as u8 & 0x3F)))
            .collect(),
    )
    .unwrap_or_default()
}

/// 旧版 Windows 云同步曾在 DPAPI 不可用时写入混淆值。
pub fn decrypt_secret(s: &str) -> String {
    crate::secret::unprotect(s).unwrap_or_else(|_| {
        if crate::secret::needs_upgrade(s) {
            let legacy = deobfuscate(s);
            if legacy.bytes().all(|b| b.is_ascii_graphic()) {
                return legacy;
            }
        }
        String::new()
    })
}

pub fn load(state: &AppState) -> CloudState {
    load_at(&state_path(state)).unwrap_or_default()
}

fn load_at(path: &std::path::Path) -> Result<CloudState, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|err| format!("云同步状态损坏: {err}")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(CloudState::default()),
        Err(err) => Err(format!("读取云同步状态失败: {err}")),
    }
}

pub fn edit<T>(state: &AppState, change: impl FnOnce(&mut CloudState) -> Result<T, String>) -> Result<T, String> {
    edit_at(&state_path(state), change)
}

fn edit_at<T>(path: &std::path::Path, change: impl FnOnce(&mut CloudState) -> Result<T, String>) -> Result<T, String> {
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = WRITE.lock().map_err(|_| "无法锁定云同步状态")?;
    let original = load_at(path)?;
    let mut cs = original.clone();
    let result = change(&mut cs)?;
    if cs == original {
        return Ok(result);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let text = serde_json::to_vec_pretty(&cs).map_err(|e| e.to_string())?;
    crate::fsutil::atomic_write(path, &text).map_err(|e| format!("写入云同步状态失败: {e}"))?;
    Ok(result)
}

pub fn access_token(cs: &CloudState) -> String {
    decrypt_secret(&cs.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_merges_settings_and_rejects_a_disconnected_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let before = edit_at(&path, |current| {
            current.session_id = "session-one".into();
            current.account = "example".into();
            current.access_token = "old-token".into();
            Ok(current.clone())
        }).unwrap();
        edit_at(&path, |current| { current.keep_per_world = 9; Ok(()) }).unwrap();
        let refresh = |current: &mut CloudState| {
            current.check_session(&before)?;
            current.access_token = "new-token".into();
            Ok(())
        };
        edit_at(&path, refresh).unwrap();
        let after = load_at(&path).unwrap();
        assert_eq!(after.keep_per_world, 9);
        assert_eq!(after.access_token, "new-token");
        edit_at(&path, |current| {
            *current = CloudState { keep_per_world: 9, ..Default::default() };
            Ok(())
        }).unwrap();
        assert!(edit_at(&path, refresh).is_err());
        assert!(!load_at(&path).unwrap().connected());
    }
}
