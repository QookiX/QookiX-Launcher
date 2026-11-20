use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct RuntimeJobs(Arc<Registry>);

struct Registry {
    relocation_pending: Mutex<bool>,
    next_id: AtomicU64,
    processes: Mutex<HashMap<(ProcessKind, String), ProcessEntry>>,
    installs: Mutex<HashMap<u64, InstallEntry>>,
    downloads: Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ProcessKind {
    Game,
    Server,
    Terracotta,
}

struct ProcessEntry {
    filesystem: bool,
    task_id: u64,
    pid: Option<u32>,
    cancel: CancellationToken,
    finished: CancellationToken,
}

struct InstallEntry {
    instance_id: String,
    cancel: CancellationToken,
}

pub struct ProcessLease {
    jobs: RuntimeJobs,
    key: (ProcessKind, String),
    pub task_id: u64,
    pub cancel: CancellationToken,
    finished: CancellationToken,
}

pub struct InstallLease {
    jobs: RuntimeJobs,
    pub task_id: u64,
    pub cancel: CancellationToken,
}

impl Default for RuntimeJobs {
    fn default() -> Self {
        Self(Arc::new(Registry {
            relocation_pending: Mutex::new(false),
            next_id: AtomicU64::new(1),
            processes: Mutex::new(HashMap::new()),
            installs: Mutex::new(HashMap::new()),
            downloads: Mutex::new(HashMap::new()),
        }))
    }
}

impl RuntimeJobs {
    /// Instances share asset and library paths. Serialize writes to the same staging file.
    pub fn download_lock(&self, staging_path: PathBuf) -> Arc<AsyncMutex<()>> {
        let mut downloads = self
            .0
            .downloads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        downloads.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = downloads.get(&staging_path).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        downloads.insert(staging_path, Arc::downgrade(&lock));
        lock
    }

    pub fn next_id(&self) -> u64 {
        self.0.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn queue_relocation(
        &self,
        schedule: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut pending = self
            .0
            .relocation_pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *pending {
            return Err("已安排数据目录迁移，请先重启启动器".into());
        }
        let processes = self
            .0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let installs = self
            .0
            .installs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !processes.is_empty() || !installs.is_empty() {
            return Err("请先关闭游戏、服务器并等待安装和文件操作结束".into());
        }
        schedule()?;
        *pending = true;
        Ok(())
    }

    pub fn reserve_game(&self, id: &str) -> Result<ProcessLease, String> {
        self.reserve_process(ProcessKind::Game, id, false)
    }

    pub fn reserve_server(&self, id: &str) -> Result<ProcessLease, String> {
        self.reserve_process(ProcessKind::Server, id, false)
    }

    pub fn reserve_terracotta(&self) -> Result<ProcessLease, String> {
        self.reserve_process(ProcessKind::Terracotta, "", false)
    }

    pub fn reserve_game_files(&self, id: &str) -> Result<ProcessLease, String> {
        self.reserve_process(ProcessKind::Game, id, true)
    }

    pub fn reserve_server_files(&self, id: &str) -> Result<ProcessLease, String> {
        self.reserve_process(ProcessKind::Server, id, true)
    }

    fn reserve_process(
        &self,
        kind: ProcessKind,
        id: &str,
        filesystem: bool,
    ) -> Result<ProcessLease, String> {
        let migration = self
            .0
            .relocation_pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *migration {
            return Err("请先重启启动器完成数据目录迁移".into());
        }
        let mut tasks = self
            .0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let key = (kind, id.to_owned());
        if let Some(active) = tasks.get(&key) {
            let subject = match kind {
                ProcessKind::Game => "该实例",
                ProcessKind::Server => "该服务器",
                ProcessKind::Terracotta => "陶瓦联机",
            };
            let status = if active.filesystem {
                "正在处理文件"
            } else {
                "正在启动、运行或退出"
            };
            return Err(format!("{subject}{status}，请稍后重试"));
        }
        let task_id = self.next_id();
        let cancel = CancellationToken::new();
        let finished = CancellationToken::new();
        tasks.insert(
            key.clone(),
            ProcessEntry {
                filesystem,
                task_id,
                pid: None,
                cancel: cancel.clone(),
                finished: finished.clone(),
            },
        );
        Ok(ProcessLease {
            jobs: self.clone(),
            key,
            task_id,
            cancel,
            finished,
        })
    }

    pub fn game_busy(&self, id: &str) -> bool {
        self.0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&(ProcessKind::Game, id.to_owned()))
            .is_some_and(|entry| !entry.filesystem)
    }

    pub fn server_running(&self, id: &str) -> bool {
        self.0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&(ProcessKind::Server, id.to_owned()))
            .is_some_and(|entry| entry.pid.is_some())
    }

    pub fn game_pids(&self) -> HashMap<String, u32> {
        self.0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|((kind, _), _)| *kind == ProcessKind::Game)
            .filter_map(|((_, id), entry)| entry.pid.map(|pid| (id.clone(), pid)))
            .collect()
    }

    pub fn request_game_shutdowns(&self) -> Vec<CancellationToken> {
        self.0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|((kind, _), entry)| *kind == ProcessKind::Game && !entry.filesystem)
            .map(|(_, entry)| {
                entry.cancel.cancel();
                entry.finished.clone()
            })
            .collect()
    }

    pub fn request_server_shutdown(&self, id: &str) -> Option<CancellationToken> {
        self.request_shutdown(ProcessKind::Server, id)
    }

    pub fn request_terracotta_shutdown(&self) -> Option<CancellationToken> {
        self.request_shutdown(ProcessKind::Terracotta, "")
    }

    fn request_shutdown(&self, kind: ProcessKind, id: &str) -> Option<CancellationToken> {
        let tasks = self
            .0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = tasks.get(&(kind, id.to_owned()))?;
        if entry.filesystem {
            return None;
        }
        entry.cancel.cancel();
        Some(entry.finished.clone())
    }

    pub fn register_install(
        &self,
        task_id: u64,
        instance_id: &str,
        parent: Option<&CancellationToken>,
    ) -> Result<InstallLease, String> {
        let migration = self
            .0
            .relocation_pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *migration {
            return Err("请先重启启动器完成数据目录迁移".into());
        }
        let mut installs = self
            .0
            .installs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if installs.contains_key(&task_id)
            || installs
                .values()
                .any(|task| task.instance_id == instance_id)
        {
            return Err("该实例已有安装任务".into());
        }
        let cancel = parent
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        installs.insert(
            task_id,
            InstallEntry {
                instance_id: instance_id.into(),
                cancel: cancel.clone(),
            },
        );
        Ok(InstallLease {
            jobs: self.clone(),
            task_id,
            cancel,
        })
    }

    pub fn install_token(&self, task_id: u64) -> CancellationToken {
        self.0
            .installs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&task_id)
            .map(|entry| entry.cancel.clone())
            .unwrap_or_default()
    }

    pub fn has_install(&self, task_id: u64) -> bool {
        self.0.installs.lock().unwrap_or_else(|error| error.into_inner()).contains_key(&task_id)
    }

    pub fn cancel_install_task(&self, task_id: u64) {
        let installs = self.0.installs.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(task) = installs.get(&task_id) {
            task.cancel.cancel();
        }
    }

    pub fn cancel_install(&self, instance_id: Option<&str>) -> Result<(), String> {
        let installs = self
            .0
            .installs
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut matching = installs
            .values()
            .filter(|entry| instance_id.is_none_or(|id| entry.instance_id == id));
        let Some(entry) = matching.next() else {
            return Ok(());
        };
        if matching.next().is_some() {
            return Err("有多个安装任务，请指定实例".into());
        }
        entry.cancel.cancel();
        Ok(())
    }
}

impl ProcessLease {
    pub fn set_pid(&self, pid: u32) {
        let mut tasks = self
            .jobs
            .0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = tasks
            .get_mut(&self.key)
            .filter(|entry| entry.task_id == self.task_id)
        {
            entry.pid = Some(pid);
        }
    }
}

impl Drop for ProcessLease {
    fn drop(&mut self) {
        let mut tasks = self
            .jobs
            .0
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if tasks
            .get(&self.key)
            .is_some_and(|entry| entry.task_id == self.task_id)
        {
            tasks.remove(&self.key);
        }
        self.finished.cancel();
    }
}

impl Drop for InstallLease {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.jobs
            .0
            .installs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.task_id);
    }
}

impl InstallLease {
    pub fn attach_instance(&self, instance: &str) -> Result<(), String> {
        self.check()?;
        let mut installs = self.jobs.0.installs.lock().unwrap_or_else(|error| error.into_inner());
        if installs.iter().any(|(id, task)| *id != self.task_id && task.instance_id == instance) {
            return Err("该实例已有安装任务".into());
        }
        let task = installs.get_mut(&self.task_id).ok_or("安装任务不存在")?;
        task.instance_id = instance.into();
        Ok(())
    }

    pub fn check(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            Err("安装已取消".into())
        } else {
            Ok(())
        }
    }

    pub async fn until_cancelled<F: std::future::Future>(&self, future: F) -> Result<F::Output, String> {
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err("安装已取消".into()),
            result = future => Ok(result),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_stop_reserves_server_until_exit_and_keeps_instance_identity_separate() {
        let jobs = RuntimeJobs::default();
        let server = jobs.reserve_server("world").unwrap();
        let game = jobs.reserve_game("world").unwrap();
        assert!(jobs.reserve_server("world").is_err());
        assert!(jobs.reserve_server_files("world").is_err());
        assert!(!jobs.server_running("world"));
        server.set_pid(27);
        let idle = jobs.request_server_shutdown("world").unwrap();
        assert!(server.cancel.is_cancelled());
        assert!(!game.cancel.is_cancelled());
        assert!(!idle.is_cancelled());
        assert!(jobs.server_running("world"));
        assert!(jobs.reserve_server("world").is_err());
        drop(server);
        assert!(idle.is_cancelled());
        let maintenance = jobs.reserve_server_files("world").unwrap();
        assert!(jobs.reserve_server("world").is_err());
        assert!(jobs.request_server_shutdown("world").is_none());
        drop(maintenance);
        assert!(jobs.reserve_server("world").is_ok());
    }

    #[test]
    fn instance_files_and_launch_cannot_overlap() {
        let jobs = RuntimeJobs::default();
        let files = jobs.reserve_game_files("one").unwrap();
        assert!(jobs.reserve_game("one").is_err());
        assert!(jobs.reserve_game_files("one").is_err());
        assert!(jobs.request_game_shutdowns().is_empty());
        let unrelated = jobs.reserve_game("two").unwrap();
        drop(files);
        let own = jobs.reserve_game("one").unwrap();
        assert!(jobs.reserve_game_files("one").is_err());
        assert!(!unrelated.cancel.is_cancelled());
        assert!(!own.cancel.is_cancelled());
    }

    #[test]
    fn concurrent_downloads_share_staging_guard_but_unrelated_files_do_not() {
        let jobs = RuntimeJobs::default();
        let first = jobs.download_lock("shared.part".into());
        let _guard = first.try_lock().unwrap();
        assert!(jobs.download_lock("shared.part".into()).try_lock().is_err());
        assert!(jobs
            .download_lock("separate.part".into())
            .try_lock()
            .is_ok());
    }

    #[test]
    fn game_stays_reserved_until_the_process_lease_finishes() {
        let jobs = RuntimeJobs::default();
        let game = jobs.reserve_game("world").unwrap();
        assert!(jobs.reserve_game("world").is_err());
        assert!(jobs.game_pids().is_empty());
        game.set_pid(25);
        let finished = jobs.request_game_shutdowns().pop().unwrap();
        assert!(game.cancel.is_cancelled());
        assert!(!finished.is_cancelled());
        assert_eq!(jobs.game_pids().get("world"), Some(&25));
        let first = game.task_id;
        drop(game);
        assert!(finished.is_cancelled());
        assert!(!jobs.game_busy("world"));
        assert_ne!(jobs.reserve_game("world").unwrap().task_id, first);
    }

    #[test]
    fn cancellation_is_bound_to_the_installation_that_requested_it() {
        let jobs = RuntimeJobs::default();
        let first = jobs.register_install(jobs.next_id(), "one", None).unwrap();
        let second = jobs.register_install(jobs.next_id(), "two", None).unwrap();
        assert!(jobs.register_install(jobs.next_id(), "one", None).is_err());
        assert!(jobs.cancel_install(None).is_err());
        jobs.cancel_install(Some("one")).unwrap();
        assert!(jobs.install_token(first.task_id).is_cancelled());
        assert!(!jobs.install_token(second.task_id).is_cancelled());
        drop(first);
        assert!(jobs.register_install(jobs.next_id(), "one", None).is_ok());
    }

    #[tokio::test]
    async fn a_pack_keeps_the_same_cancellation_id_from_metadata_to_instance() {
        let jobs = RuntimeJobs::default();
        let pack = jobs.register_install(jobs.next_id(), "@pack-prepare", None).unwrap();
        let other = jobs.register_install(jobs.next_id(), "unrelated", None).unwrap();
        pack.attach_instance("new-pack").unwrap();
        assert!(jobs.register_install(jobs.next_id(), "new-pack", None).is_err());
        let (metadata, ()) = tokio::join!(
            pack.until_cancelled(std::future::pending::<()>()),
            async { jobs.cancel_install_task(pack.task_id); },
        );
        assert!(metadata.is_err());
        assert!(jobs.install_token(pack.task_id).is_cancelled());
        assert!(!other.cancel.is_cancelled());
    }
}
