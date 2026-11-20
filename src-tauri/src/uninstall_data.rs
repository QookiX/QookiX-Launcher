use crate::settings;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const BUNDLE_ID: &str = "cn.swkj1.qookix-launcher";
const APP_FILES: &[&str] = &[
    "qookix_auth.json",
    "shimu_auth.json",
    "frp_auth.json",
    "ygg_oauth_pending.json",
    "qookix.db-shm",
    "qookix.db-wal",
    "qookix.db",
];
const DEFAULT_FILES: &[&str] = &[
    "accounts.json", "settings.json", "pins.json", "playtime.json", "instance_groups.json",
    "java-cache.json", "settings.json.migrated.bak",
];
const DEFAULT_DIRS: &[&str] = &[
    "authlib-injector",
    "cloud_sync",
    "timemachine",
    "instances",
    "servers",
    "libraries",
    "assets",
    "versions",
    "logs",
    "runtimes",
    "skins",
    "frp",
    "shimu",
    "cache",
    "downloads",
    "crash-reports",
    "backups",
];

#[derive(Debug)]
struct CleanupPlan {
    root: PathBuf,
    bootstrap: PathBuf,
    entries: Vec<PathBuf>,
    custom: bool,
}

fn regular_metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if !crate::fsutil::is_link_or_reparse_point(&meta) => Ok(Some(meta)),
        Ok(_) => Ok(None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("{}: {err}", path.display())),
    }
}

fn require_dir(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || settings::is_disk_root(path)
        || settings::is_system_dir(path)
        || !regular_metadata(path)?.is_some_and(|meta| meta.is_dir())
    {
        return Err(format!("无法确认目录属于 QookiX: {}", path.display()));
    }
    let full = path.canonicalize().map_err(|err| err.to_string())?;
    if settings::is_disk_root(&full) || settings::is_system_dir(&full) {
        return Err("选定的是系统目录".into());
    }
    Ok(full)
}

fn recognized_database(root: &Path) -> bool {
    use rusqlite::{Connection, OpenFlags};
    let path = root.join("qookix.db");
    if !regular_metadata(&path)
        .ok()
        .flatten()
        .is_some_and(|meta| meta.is_file())
    {
        return false;
    }
    let Ok(connection) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return false;
    };
    connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('accounts', 'settings', 'pins', 'playtime')",
        [], |row| row.get::<_, u32>(0),
    ).is_ok_and(|count| count == 4)
}

fn owned_profiles<T: serde::de::DeserializeOwned>(
    root: &Path,
    marker: &str,
    id: impl Fn(&T) -> &str,
) -> Result<Vec<PathBuf>, String> {
    if !regular_metadata(root)?.is_some_and(|meta| meta.is_dir()) {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(root).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let name = entry.file_name();
        let marker = path.join(marker);
        if !regular_metadata(&path)?.is_some_and(|meta| meta.is_dir())
            || !regular_metadata(&marker)?
                .is_some_and(|meta| meta.is_file() && meta.len() <= 1024 * 1024)
        {
            continue;
        }
        let owned = fs::read(&marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<T>(&bytes).ok())
            .is_some_and(|profile| name == id(&profile));
        if owned {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn make_plan(
    bootstrap: &Path,
    default: &Path,
    configured: Option<&Path>,
) -> Result<CleanupPlan, String> {
    let bootstrap = require_dir(bootstrap)?;
    let root = require_dir(configured.unwrap_or(&bootstrap))?;
    let custom = default.canonicalize().ok().as_ref() != Some(&root);
    if custom && !recognized_database(&root) {
        return Err(format!(
            "在 {} 未找到可验证的 QookiX 数据库，已保留此目录。",
            root.display()
        ));
    }
    let mut entries = Vec::new();
    if custom {
        for (folder, file) in [
            ("authlib-injector", "authlib-injector.jar"),
            ("cloud_sync", "state.json"),
            ("timemachine", "state.json"),
        ] {
            let folder = root.join(folder);
            if regular_metadata(&folder)?.is_some_and(|meta| meta.is_dir()) {
                let target = folder.join(file);
                if regular_metadata(&target)?.is_some_and(|meta| meta.is_file()) {
                    entries.push(target);
                }
            }
        }
        entries.extend(owned_profiles::<crate::models::Instance>(
            &root.join("instances"),
            "qookix.json",
            |model| &model.id,
        )?);
        entries.extend(owned_profiles::<crate::models::ServerConfig>(
            &root.join("servers"),
            "server.json",
            |model| &model.id,
        )?);
    } else {
        entries.extend(DEFAULT_DIRS.iter().chain(DEFAULT_FILES).map(|name| root.join(name)));
    }
    entries.extend(APP_FILES.iter().map(|name| root.join(name)));
    entries.push(bootstrap.join(settings::DATA_ROOT_POINTER));
    Ok(CleanupPlan {
        root,
        bootstrap,
        entries,
        custom,
    })
}

fn remove_owned_entry(root: &Path, target: &Path) -> Result<(), String> {
    let Some(meta) = regular_metadata(target)? else {
        return Ok(());
    };
    let canonical = target.canonicalize().map_err(|err| err.to_string())?;
    if canonical == root
        || !canonical.starts_with(root)
        || canonical.to_string_lossy().to_lowercase() != target.to_string_lossy().to_lowercase()
    {
        return Err(format!("已保留无法确认归属的路径 {}", target.display()));
    }
    let result = if meta.is_dir() {
        fs::remove_dir_all(target)
    } else {
        fs::remove_file(target)
    };
    result.map_err(|err| format!("无法删除 {}: {err}", target.display()))
}

impl CleanupPlan {
    fn remove(&self) -> Result<(), String> {
        let pointer = self.bootstrap.join(settings::DATA_ROOT_POINTER);
        for target in &self.entries {
            let root = if target == &pointer {
                &self.bootstrap
            } else {
                &self.root
            };
            remove_owned_entry(root, target)?;
        }
        Ok(())
    }
}

fn confirm(plan: &CleanupPlan, chinese: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO,
    };
    let clean = plan.root.to_string_lossy();
    let message = match (chinese, plan.custom) {
        (true, true) => format!("删除 {clean} 中可识别的 QookiX 设置、账号、实例、托管服和专用数据？\n\n共享游戏资源和其他文件会保留。"),
        (true, false) => format!("删除 {clean} 中的 QookiX 设置、账号、实例、托管服和下载资源？\n\n未识别的文件会保留。"),
        (false, true) => format!("Remove the recognized QookiX settings, accounts, instances, servers and app data from {clean}?\n\nShared game resources and other files are kept."),
        (false, false) => format!("Remove the QookiX settings, accounts, instances, servers and downloaded resources from {clean}?\n\nUnrecognized files are kept."),
    };
    let message: Vec<_> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<_> = "QookiX Launcher".encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        ) == IDYES
    }
}

pub fn run(chinese: bool) -> Result<(), String> {
    // A separately elevated account can have a different AppData and user token.
    if unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } != 0 {
        return Err("请以启动器所属的普通 Windows 用户身份执行数据清理".into());
    }
    let default = PathBuf::from(settings::default_root());
    let bootstrap = settings::bootstrap_root(settings::portable_data_dir());
    if !bootstrap.try_exists().map_err(|err| err.to_string())? {
        return Ok(());
    }
    let bootstrap = require_dir(&bootstrap)?;
    let _lock = crate::data_root_migration::lock_for_uninstall(&bootstrap)?;
    let configured = settings::configured_data_dir(&bootstrap);
    let plan = make_plan(&bootstrap, &default, configured.as_deref())?;
    if !confirm(&plan, chinese) {
        return Ok(());
    }
    plan.remove()?;
    if let Some(base) = directories::BaseDirs::new() {
        let bundle = base.data_local_dir().join(BUNDLE_ID);
        if regular_metadata(&bundle)?.is_some_and(|meta| meta.is_dir()) {
            let bundle = require_dir(&bundle)?;
            remove_owned_entry(&bundle, &bundle.join("EBWebView"))?;
            let _ = fs::remove_dir(&bundle);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_cleanup_keeps_unrelated_files_and_reparse_targets() {
        let sandbox = tempfile::tempdir().unwrap();
        let root = sandbox.path().join("custom");
        let shared = sandbox.path().join("shared");
        let bootstrap = sandbox.path().join("bootstrap");
        for directory in [&root, &shared, &bootstrap, &root.join("libraries")] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(shared.join("keep.txt"), b"outside").unwrap();
        junction::create(&shared, root.join("cloud_sync")).unwrap();
        fs::write(root.join("libraries/keep.jar"), b"shared game content").unwrap();
        fs::write(root.join("unrelated.txt"), b"unrelated").unwrap();
        fs::write(root.join("settings.json"), b"external settings").unwrap();
        fs::create_dir(root.join("authlib-injector")).unwrap();
        fs::write(root.join("authlib-injector/owned.txt"), b"outside launcher files").unwrap();
        fs::write(root.join("authlib-injector/authlib-injector.jar"), b"fixture").unwrap();
        assert!(make_plan(&bootstrap, &bootstrap, Some(&root)).is_err());
        drop(crate::db::initialize(&root).unwrap());
        settings::write_data_root_pointer(&bootstrap, &root).unwrap();
        let plan = make_plan(&bootstrap, &bootstrap, Some(&root)).unwrap();
        plan.remove().unwrap();
        assert!(!root.join("qookix.db").exists());
        assert!(root.join("libraries/keep.jar").is_file());
        assert!(root.join("unrelated.txt").is_file());
        assert!(root.join("settings.json").is_file());
        assert!(root.join("authlib-injector/owned.txt").is_file());
        assert!(!root.join("authlib-injector/authlib-injector.jar").exists());
        assert!(shared.join("keep.txt").is_file());
        assert!(!bootstrap.join(settings::DATA_ROOT_POINTER).exists());
        junction::delete(root.join("cloud_sync")).unwrap();
    }
}
