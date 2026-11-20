use crate::models::Settings;
use crate::shimu::ShimuAuth;
use crate::yggdrasil_oauth::YggOauthPending;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::Semaphore;

pub struct AppState {
    pub root: PathBuf,
    pub settings: RwLock<Settings>,
    /// 应用数据库（SQLite）：账号 / 设置 / 首页固定 / 按天游玩时长
    pub db: Mutex<rusqlite::Connection>,
    pub client: reqwest::Client,
    /// Download concurrency limit
    #[allow(dead_code)]
    pub semaphore: Arc<Semaphore>,
    pub jobs: crate::runtime_jobs::RuntimeJobs,
    /// Pending Microsoft device-code flow (if any)
    pub ms_flow: Arc<Mutex<Option<crate::models::MsFlow>>>,
    /// 拾木空间（mcimg）OAuth 登录态：待校验 state + access_token + 用户资料
    pub shimu: Mutex<ShimuAuth>,
    /// LittleSkin OAuth2（Yggdrasil Connect）进行中的流程：待校验 state + 临时 OAuth token
    pub ygg_oauth: Mutex<YggOauthPending>,
    /// Cached Java detection results (unix seconds, list)
    pub java_cache: Mutex<Option<(u64, Vec<crate::models::JavaInfo>)>>,
    /// Currently running Terracotta (陶瓦联机) session, if any
    pub terracotta: Mutex<Option<crate::terracotta::TerracottaSession>>,
    /// 已检查的更新候选和待安装的签名安装包
    pub(crate) pending_update: Mutex<crate::updater::UpdateCache>,
    /// 局域网同步服务（运行中时为 Some，关闭后为 None）
    pub lan_sync: Mutex<Option<crate::lan_sync::LanSyncServer>>,
    /// QookiX 账号（可选）：好友与陶瓦联机房间目录
    pub qookix: Mutex<crate::qookix::QookixSession>,
    /// 令牌刷新闸门：并发 401 时只放一个请求去刷新，其余等新令牌
    pub qookix_refresh: tokio::sync::Mutex<()>,
}

impl AppState {
    pub fn instances_dir(&self) -> PathBuf {
        self.root.join("instances")
    }
    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }
    #[allow(dead_code)]
    pub fn assets_indexes_dir(&self) -> PathBuf {
        self.root.join("assets").join("indexes")
    }
    #[allow(dead_code)]
    pub fn assets_objects_dir(&self) -> PathBuf {
        self.root.join("assets").join("objects")
    }
    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn servers_dir(&self) -> PathBuf {
        self.root.join("servers")
    }
    #[allow(dead_code)]
    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    #[allow(dead_code)]
    pub fn accounts_path(&self) -> PathBuf {
        self.root.join("accounts.json")
    }

    pub fn next_task_id(&self) -> u64 {
        self.jobs.next_id()
    }

    // ---- Lock helpers: recover from poison instead of panicking -------------
    // A Mutex/RwLock is poisoned when a thread panicked while holding it.
    // Calling .unwrap() on the lock result would panic again, cascading the
    // failure to every subsequent caller and bricking the backend.  These
    // helpers recover the guard via PoisonError::into_inner(), so a single
    // panic doesn't take down the whole app.

    /// Read guard for `settings`, recovering from lock poison.
    pub fn settings(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
        self.settings.read().unwrap_or_else(|e| e.into_inner())
    }
    /// Write guard for `settings`, recovering from lock poison.
    pub fn settings_mut(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
        self.settings.write().unwrap_or_else(|e| e.into_inner())
    }
    /// Guard for `db`, recovering from lock poison.
    pub fn db(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Guard for `qookix`, recovering from lock poison.
    pub fn qookix(&self) -> std::sync::MutexGuard<'_, crate::qookix::QookixSession> {
        self.qookix.lock().unwrap_or_else(|e| e.into_inner())
    }
}
