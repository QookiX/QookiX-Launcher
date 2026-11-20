use crate::{fsutil, settings};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const JOURNAL: &str = "data-root-migration.json";
const LOCK: &str = ".data-root.lock";
const MARKER: &str = ".data-root-migration";
const ERROR: &str = "data-root-migration-error.txt";
const BOOTSTRAP_FILES: [&str; 4] = [JOURNAL, LOCK, ERROR, settings::DATA_ROOT_POINTER];
const DATABASE_FILES: [&str; 4] = [
    "qookix.db",
    "qookix.db-shm",
    "qookix.db-wal",
    "qookix.db-journal",
];

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MigrationMode {
    Copy,
    Move,
}

#[derive(Deserialize, Serialize)]
struct Migration {
    id: String,
    source: PathBuf,
    target: PathBuf,
    mode: MigrationMode,
}

fn is_bootstrap_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| BOOTSTRAP_FILES.contains(&name))
}

fn read_plan(bootstrap: &Path) -> io::Result<Option<Migration>> {
    match fs::read(bootstrap.join(JOURNAL)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn schedule(
    bootstrap: &Path,
    source: &Path,
    target: &Path,
    mode: MigrationMode,
) -> Result<(), String> {
    let source = source
        .canonicalize()
        .map_err(|error| format!("无法找到当前数据目录: {error}"))?;
    let target = target
        .canonicalize()
        .map_err(|error| format!("无法找到目标数据目录: {error}"))?;
    let plan = Migration {
        id: Uuid::new_v4().to_string(),
        source,
        target,
        mode,
    };
    check_layout(&plan).map_err(|error| error.to_string())?;
    ensure_target_empty(&plan.target).map_err(|error| error.to_string())?;
    if let Some(previous) = read_plan(bootstrap).map_err(|error| error.to_string())? {
        if previous.source == plan.source && previous.target == plan.target && previous.mode == mode
        {
            return Ok(());
        }
        return Err("已有数据迁移安排，请先重启完成或取消该操作".into());
    }
    let bytes = serde_json::to_vec_pretty(&plan).map_err(|error| error.to_string())?;
    fsutil::atomic_write(&bootstrap.join(JOURNAL), &bytes)
        .map_err(|error| format!("无法保存数据迁移计划: {error}"))
}

/// The shared lock stays alive throughout the app run. An interrupted copy can
/// be retried only when no launcher using this bootstrap root still holds it.
pub fn open_bootstrap(bootstrap: &Path) -> io::Result<File> {
    fs::create_dir_all(bootstrap)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(bootstrap.join(LOCK))?;
    if bootstrap.join(JOURNAL).try_exists()? {
        let mut acquired = false;
        for _ in 0..40 {
            match lock.try_lock() {
                Ok(()) => {
                    acquired = true;
                    break;
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(75))
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error),
            }
        }
        if acquired {
            if let Err(error) = run_pending(bootstrap) {
                let message = format!("数据目录迁移未完成: {error}");
                eprintln!("{message}");
                let _ = fsutil::atomic_write(&bootstrap.join(ERROR), message.as_bytes());
            } else {
                let _ = fs::remove_file(bootstrap.join(ERROR));
            }
            lock.unlock()?;
        }
    }
    lock.lock_shared()?;
    Ok(lock)
}

pub fn ensure_no_pending(bootstrap: &Path) -> Result<(), String> {
    if read_plan(bootstrap)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("请先完成或取消已安排的数据目录迁移".into());
    }
    Ok(())
}

#[cfg(windows)]
pub fn lock_for_uninstall(bootstrap: &Path) -> Result<File, String> {
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(bootstrap.join(LOCK)).map_err(|err| err.to_string())?;
    lock.try_lock().map_err(|err| format!("请退出正在使用此数据目录的启动器: {err}"))?;
    ensure_no_pending(bootstrap)?;
    Ok(lock)
}

pub fn cancel_failed(bootstrap: &Path) -> Result<(), String> {
    if !bootstrap
        .join(ERROR)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("当前没有执行失败的迁移".into());
    }
    let Some(plan) = read_plan(bootstrap).map_err(|error| error.to_string())? else {
        return Ok(());
    };
    if fs::read_to_string(plan.target.join(MARKER)).ok().as_deref() == Some(&plan.id) {
        return Err("目标数据已准备完成，请重新启动以结束迁移".into());
    }
    fs::remove_file(bootstrap.join(JOURNAL)).map_err(|error| error.to_string())?;
    let _ = fs::remove_file(bootstrap.join(ERROR));
    Ok(())
}

pub fn latest_error(bootstrap: &Path) -> Option<String> {
    fs::read_to_string(bootstrap.join(ERROR)).ok()
}

fn check_layout(plan: &Migration) -> io::Result<()> {
    if !plan.source.is_absolute()
        || !plan.target.is_absolute()
        || settings::is_disk_root(&plan.source)
        || settings::is_system_dir(&plan.source)
        || settings::is_disk_root(&plan.target)
        || settings::is_system_dir(&plan.target)
        || plan.target.starts_with(&plan.source)
        || plan.source.starts_with(&plan.target)
    {
        return Err(io::Error::other("源目录和目标目录必须是互不包含的普通目录"));
    }
    Ok(())
}

fn ensure_target_empty(target: &Path) -> io::Result<()> {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() && !fsutil::is_link_or_reparse_point(&meta) => {
            if fs::read_dir(target)?.next().transpose()?.is_some() {
                return Err(io::Error::other("目标目录不为空"));
            }
        }
        Ok(_) => return Err(io::Error::other("目标目录不是普通目录")),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn run_pending(bootstrap: &Path) -> io::Result<()> {
    let Some(plan) = read_plan(bootstrap)? else {
        return Ok(());
    };
    check_layout(&plan)?;
    let existing_root =
        settings::configured_data_dir(bootstrap).unwrap_or_else(|| bootstrap.to_path_buf());
    let existing_root = existing_root.canonicalize().unwrap_or(existing_root);
    if existing_root != plan.source && existing_root != plan.target {
        return Err(io::Error::other("启动目录已被切换，无法继续该迁移"));
    }

    let marker = plan.target.join(MARKER);
    let id = plan.id.as_str();
    let completed_copy = fs::read_to_string(&marker).ok().as_deref() == Some(id);
    if !completed_copy {
        if existing_root == plan.target {
            return Err(io::Error::other("目标数据缺少对应的迁移完成标记"));
        }
        ensure_target_empty(&plan.target)?;
        let parent = plan
            .target
            .parent()
            .ok_or_else(|| io::Error::other("目标目录没有父目录"))?;
        let stage = tempfile::Builder::new()
            .prefix(".qookix-migration-")
            .tempdir_in(parent)?;
        copy_root(&plan, stage.path())?;
        fsutil::atomic_write(&stage.path().join(MARKER), id.as_bytes())?;

        // remove_dir is intentionally non-recursive and fails if a new file appeared.
        match fs::remove_dir(&plan.target) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(stage.path(), &plan.target)?;
    }

    settings::write_data_root_pointer(bootstrap, &plan.target).map_err(io::Error::other)?;
    if plan.mode == MigrationMode::Move {
        clean_source(&plan.source)?;
    }
    fs::remove_file(bootstrap.join(JOURNAL))?;
    let _ = fs::remove_file(marker);
    Ok(())
}

fn copy_root(plan: &Migration, target: &Path) -> io::Result<()> {
    let source = &plan.source;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        if !is_bootstrap_file(&path)
            && !entry
                .file_name()
                .to_str()
                .is_some_and(|name| DATABASE_FILES.contains(&name) || name == MARKER)
        {
            copy_entry(plan, &path, &target.join(entry.file_name()))?;
        }
    }
    let source_db = crate::db::db_path(source);
    if source_db.try_exists()? {
        let source = Connection::open_with_flags(source_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(io::Error::other)?;
        source
            .busy_timeout(Duration::from_secs(4))
            .map_err(io::Error::other)?;
        let target = crate::db::db_path(target);
        let target_path = target
            .to_str()
            .ok_or_else(|| io::Error::other("数据库目标路径不是 UTF-8"))?;
        source
            .execute("VACUUM INTO ?1", [target_path])
            .map_err(io::Error::other)?;
        let check = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(io::Error::other)?;
        let status: String = check
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(io::Error::other)?;
        if status != "ok" {
            return Err(io::Error::other(format!("迁移后数据库校验失败: {status}")));
        }
    }
    Ok(())
}

fn migrated_link(plan: &Migration, link: PathBuf) -> PathBuf {
    if !link.is_absolute() {
        return link;
    }
    let normal = |path: &Path| path.components().all(|part| !matches!(part, std::path::Component::ParentDir));
    if let Ok(relative) = link.strip_prefix(&plan.source) {
        if normal(relative) {
            return plan.target.join(relative);
        }
    }
    #[cfg(windows)]
    {
        fn conventional(path: &Path) -> String {
            let path = path.to_string_lossy().replace('/', "\\");
            if let Some(unc) = path.strip_prefix("\\\\?\\UNC\\") {
                return format!("\\\\{unc}");
            }
            path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
        }
        let source = conventional(&plan.source);
        let path = conventional(&link);
        if path.get(..source.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(&source)) {
            let tail = &path[source.len()..];
            if tail.is_empty() { return plan.target.clone(); }
            if let Some(tail) = tail.strip_prefix('\\') {
                let relative = Path::new(tail);
                if normal(relative) && relative.components().all(|part| matches!(part, std::path::Component::Normal(_))) {
                    return plan.target.join(relative);
                }
            }
        }
    }
    link
}

fn copy_entry(plan: &Migration, source: &Path, target: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if fsutil::is_link_or_reparse_point(&metadata) {
        let link = migrated_link(plan, fs::read_link(source)?);
        #[cfg(unix)]
        std::os::unix::fs::symlink(link, target)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::{symlink_dir, symlink_file, MetadataExt};
            if metadata.file_attributes() & 0x10 != 0 {
                symlink_dir(&link, target).or_else(|error| {
                    if link.is_absolute() {
                        junction::create(&link, target)
                    } else {
                        Err(error)
                    }
                })?;
            } else {
                symlink_file(&link, target)?;
            }
        }
    } else if metadata.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_entry(plan, &entry.path(), &target.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        fs::copy(source, target)?;
    } else {
        return Err(io::Error::other(format!(
            "无法迁移特殊文件 {}",
            source.display()
        )));
    }
    Ok(())
}

fn clean_source(source: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let path = entry?.path();
        if is_bootstrap_file(&path) || path.file_name().is_some_and(|name| name == "settings.json")
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let directory = metadata.is_dir();
        if directory {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_an_absolute_internal_link_does_not_leave_it_pointing_at_the_removed_source() {
        let fixture = tempfile::tempdir().unwrap();
        let (boot, target, shared) = (fixture.path().join("boot"), fixture.path().join("target"), fixture.path().join("shared"));
        for path in [&boot, &target, &shared] { fs::create_dir(path).unwrap(); }
        let payload = boot.join("data");
        fs::create_dir(&payload).unwrap();
        fs::write(payload.join("owned.txt"), "owned").unwrap();
        fs::write(shared.join("outside.txt"), "shared").unwrap();
        #[cfg(windows)]
        let create_link = junction::create::<&Path, &Path>;
        #[cfg(unix)]
        let create_link = std::os::unix::fs::symlink::<&Path, &Path>;
        let alias = boot.join("alias");
        let outside = boot.join("external");
        create_link(payload.as_path(), alias.as_path()).unwrap();
        create_link(shared.as_path(), outside.as_path()).unwrap();
        schedule(&boot, &boot, &target, MigrationMode::Move).unwrap();
        run_pending(&boot).unwrap();
        assert!(!payload.exists());
        assert_eq!(fs::read_to_string(target.join("alias/owned.txt")).unwrap(), "owned");
        assert_eq!(fs::read_to_string(target.join("external/outside.txt")).unwrap(), "shared");
    }

    #[test]
    fn interruption_before_commit_preserves_source_and_can_retry_same_plan() {
        let fixture = tempfile::tempdir().unwrap();
        let boot = fixture.path().join("current");
        let target = fixture.path().join("new");
        fs::create_dir(&boot).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(boot.join("save"), "existing").unwrap();
        let conn = crate::db::open(&boot).unwrap();
        crate::db::set_setting(&conn, "fixture", "persisted").unwrap();
        fs::write(boot.join("qookix.db-fixture"), "extra").unwrap();
        schedule(&boot, &boot, &target, MigrationMode::Move).unwrap();
        fs::write(target.join("arrived"), "don't erase").unwrap();

        assert!(run_pending(&boot).is_err());
        assert_eq!(settings::configured_data_dir(&boot), None);
        assert!(boot.join("save").is_file());
        assert!(boot.join(JOURNAL).is_file());
        assert_eq!(
            fs::read_to_string(target.join("arrived")).unwrap(),
            "don't erase"
        );
        fs::remove_file(target.join("arrived")).unwrap();
        drop(conn);

        run_pending(&boot).unwrap();
        assert!(!boot.join("save").exists());
        assert!(!boot.join(JOURNAL).exists());
        assert_eq!(
            settings::configured_data_dir(&boot),
            Some(target.canonicalize().unwrap())
        );
        assert_eq!(fs::read_to_string(target.join("save")).unwrap(), "existing");
        let conn = crate::db::open(&target).unwrap();
        assert_eq!(
            crate::db::get_setting(&conn, "fixture").as_deref(),
            Some("persisted")
        );
    }

    #[test]
    fn recovers_after_staged_target_was_committed_before_pointer() {
        let fixture = tempfile::tempdir().unwrap();
        let boot = fixture.path().join("boot");
        let target = fixture.path().join("target");
        fs::create_dir(&boot).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(boot.join("data"), "original").unwrap();
        schedule(&boot, &boot, &target, MigrationMode::Copy).unwrap();
        let plan = read_plan(&boot).unwrap().unwrap();
        fs::write(target.join("data"), "staged data").unwrap();
        fs::write(target.join(MARKER), plan.id.to_string()).unwrap();

        run_pending(&boot).unwrap();
        assert_eq!(fs::read_to_string(boot.join("data")).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(target.join("data")).unwrap(),
            "staged data"
        );
        assert_eq!(settings::configured_data_dir(&boot), Some(plan.target));
        assert!(!target.join(MARKER).exists());
    }
}
