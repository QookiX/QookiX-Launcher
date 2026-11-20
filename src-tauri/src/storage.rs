use crate::state::AppState;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 单个存储分类的统计结果
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StorageCategory {
    pub key: String,
    pub label: String,
    pub size: u64,
    pub files: u64,
}

/// 单个游戏实例的大小
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct InstanceStorage {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub files: u64,
}

/// 存储统计整体结果
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StorageStats {
    pub categories: Vec<StorageCategory>,
    /// 每个游戏实例的单独大小
    pub instances: Vec<InstanceStorage>,
    /// 每个托管服务器实例的单独大小
    pub servers: Vec<InstanceStorage>,
    pub total: u64,
    pub instance_count: u64,
    pub server_count: u64,
    /// 上次更新时间（unix 秒）
    pub updated_at: u64,
    /// 是否为磁盘缓存数据（而非本次实时扫描）
    pub cached: bool,
}

/// 清除缓存的结果
#[derive(Serialize)]
pub struct CacheClearResult {
    /// 释放的空间（字节）
    pub freed: u64,
    pub java_download_in_use: bool,
    pub errors: Vec<String>,
}

/// 统计结果缓存文件名（位于数据根目录）
const CACHE_FILE: &str = "storage-cache.json";

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 递归统计目录大小与文件数。
/// `exclude` 中的路径（含其子树）会被跳过 —— 用于统计启动器程序时排除数据目录。
fn dir_size(path: &Path, files: &mut u64, exclude: &[PathBuf]) -> u64 {
    let mut total = 0u64;
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if exclude.iter().any(|e| p.starts_with(e)) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if crate::fsutil::is_link_or_reparse_point(&meta) {
            continue;
        }
        if meta.is_dir() {
            total += dir_size(&p, files, exclude);
        } else if meta.is_file() {
            *files += 1;
            total += meta.len();
        }
    }
    total
}

fn scan_instances(
    path: &Path,
    metadata_name: &str,
    key: &str,
    label: &str,
) -> (StorageCategory, Vec<InstanceStorage>) {
    let mut category = StorageCategory {
        key: key.into(),
        label: label.into(),
        size: 0,
        files: 0,
    };
    let mut instances = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if crate::fsutil::is_link_or_reparse_point(&meta) {
                continue;
            }
            if meta.is_file() {
                category.size += meta.len();
                category.files += 1;
            } else if meta.is_dir() {
                let path = entry.path();
                let id = entry.file_name().to_string_lossy().into_owned();
                let name = std::fs::read_to_string(path.join(metadata_name))
                    .ok()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                    .and_then(|value| value.get("name")?.as_str().map(str::to_owned))
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| id.clone());
                let mut files = 0;
                let size = dir_size(&path, &mut files, &[]);
                category.size += size;
                category.files += files;
                instances.push(InstanceStorage {
                    id,
                    name,
                    size,
                    files,
                });
            }
        }
    }
    instances.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.id.cmp(&b.id)));
    (category, instances)
}

/// 扫描数据目录，得到各分类的大小统计
pub fn scan(state: &AppState) -> StorageStats {
    let root = &state.root;
    let (instance_category, instances) = scan_instances(
        &state.instances_dir(),
        "qookix.json",
        "instances",
        "游戏实例",
    );
    let (server_category, servers) =
        scan_instances(&state.servers_dir(), "server.json", "servers", "服务器实例");
    let mut total = instance_category.size + server_category.size;
    let mut categories = vec![instance_category, server_category];

    let mut add_dir = |key: &str, label: &str, dir: PathBuf, exclude: &[PathBuf]| {
        let mut files = 0u64;
        let size = dir_size(&dir, &mut files, exclude);
        categories.push(StorageCategory {
            key: key.into(),
            label: label.into(),
            size,
            files,
        });
        total += size;
    };

    // 启动器共享数据
    add_dir("libraries", "库文件", state.libraries_dir(), &[]);
    add_dir("assets", "资源文件", state.assets_dir(), &[]);
    add_dir("versions", "版本文件", state.versions_dir(), &[]);
    // Java 运行时：排除其中的下载临时目录（downloads 归入"缓存"分类）
    let runtime_dl = crate::java::download_cache_dir(root);
    add_dir(
        "runtime",
        "Java 运行时",
        root.join("runtimes"),
        &[runtime_dl.clone()],
    );
    add_dir("logs", "日志", state.logs_dir(), &[]);

    // 缓存：可安全清理的内容（Java 下载临时目录 + Java 检测缓存 + 本统计缓存）
    let mut cache_files = 0u64;
    let mut cache_size = 0u64;
    if runtime_dl.exists() {
        cache_size += dir_size(&runtime_dl, &mut cache_files, &[]);
    }
    for cf in [root.join("java-cache.json"), cache_path(root)] {
        if cf.exists() {
            if let Ok(meta) = cf.metadata() {
                cache_files += 1;
                cache_size += meta.len();
            }
        }
    }
    categories.push(StorageCategory {
        key: "cache".into(),
        label: "缓存".into(),
        size: cache_size,
        files: cache_files,
    });
    total += cache_size;

    // 其他：数据根下未被上面覆盖的条目（设置、账号、皮肤等杂项文件/目录）
    let mut other_files = 0u64;
    let mut other_size = 0u64;
    if let Ok(rd) = std::fs::read_dir(root) {
        let covered_dirs = [
            state.instances_dir(),
            state.libraries_dir(),
            state.assets_dir(),
            state.versions_dir(),
            root.join("runtimes"),
            state.logs_dir(),
            state.servers_dir(),
        ];
        for entry in rd.flatten() {
            let p = entry.path();
            let covered = covered_dirs.iter().any(|dir| p.starts_with(dir));
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // 排除统计缓存与 Java 检测缓存（均计入"缓存"分类）
            if covered || fname == CACHE_FILE || fname == "java-cache.json" {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if crate::fsutil::is_link_or_reparse_point(&meta) {
                    continue;
                }
                if meta.is_dir() {
                    other_size += dir_size(&p, &mut other_files, &[]);
                } else if meta.is_file() {
                    other_files += 1;
                    other_size += meta.len();
                }
            }
        }
    }
    categories.push(StorageCategory {
        key: "other".into(),
        label: "其他数据".into(),
        size: other_size,
        files: other_files,
    });
    total += other_size;

    // 启动器程序本体：只统计当前 exe 文件本身。
    // 绝不能递归统计 exe 所在目录——开发模式下那是整个 Rust 的 target 构建缓存，
    // 安装模式下目录里可能还包含/是数据根，会把所有游戏数据错误地计入"启动器程序"。
    let mut launcher_files = 0u64;
    let launcher_size = std::env::current_exe()
        .ok()
        .filter(|exe| exe.is_file())
        .map(|exe| {
            launcher_files += 1;
            exe.metadata().map(|m| m.len()).unwrap_or(0)
        })
        .unwrap_or(0);
    categories.push(StorageCategory {
        key: "launcher".into(),
        label: "启动器程序".into(),
        size: launcher_size,
        files: launcher_files,
    });
    total += launcher_size;

    // 去掉无数据的分类，保持列表干净
    categories.retain(|c| c.size > 0);

    StorageStats {
        categories,
        instance_count: instances.len() as u64,
        server_count: servers.len() as u64,
        instances,
        servers,
        total,
        updated_at: now_secs(),
        cached: false,
    }
}

fn cache_path(root: &Path) -> PathBuf {
    root.join(CACHE_FILE)
}

fn load_cache(root: &Path) -> Option<StorageStats> {
    let data = std::fs::read_to_string(cache_path(root)).ok()?;
    serde_json::from_str::<StorageStats>(&data)
        .ok()
        .map(|mut s| {
            s.cached = true;
            s
        })
}

fn save_cache(root: &Path, stats: &StorageStats) {
    if let Ok(json) = serde_json::to_string(stats) {
        let p = cache_path(root);
        crate::fsutil::fs_best_effort(
            "write",
            &p,
            crate::fsutil::atomic_write(&p, json.as_bytes()),
        );
    }
}

/// 获取存储统计：优先返回上次扫描的缓存，无缓存时执行实时扫描
pub fn get_storage_stats(state: &AppState) -> StorageStats {
    let cache_lock = state.jobs.download_lock(cache_path(&state.root));
    let _guard = cache_lock.blocking_lock();
    if let Some(cached) = load_cache(&state.root) {
        return cached;
    }
    let stats = scan(state);
    save_cache(&state.root, &stats);
    stats
}

/// 强制重新扫描并保存缓存
pub fn refresh_storage_stats(state: &AppState) -> StorageStats {
    let cache_lock = state.jobs.download_lock(cache_path(&state.root));
    let _guard = cache_lock.blocking_lock();
    let stats = scan(state);
    save_cache(&state.root, &stats);
    stats
}

/// 清除可安全清理的缓存（Java 下载临时文件、Java 检测缓存、本统计缓存、
/// 内容描述翻译缓存）。不触碰任何实例、库、资源、版本等游戏数据。
pub fn clear_cache(state: &AppState) -> CacheClearResult {
    let mut result = CacheClearResult {
        freed: 0,
        java_download_in_use: false,
        errors: Vec::new(),
    };
    let stat_cache = cache_path(&state.root);
    let cache_lock = state.jobs.download_lock(stat_cache.clone());
    let _guard = cache_lock.blocking_lock();
    clear_java_download_cache(&state.jobs, &state.root, &mut result);

    for path in [state.root.join("java-cache.json"), stat_cache] {
        match remove_cache_file(&path) {
            Ok(freed) => result.freed += freed,
            Err(err) => result.errors.push(format!("{}: {err}", path.display())),
        }
    }
    match crate::translate::clear_cache(state, None) {
        Ok(freed) => result.freed += freed,
        Err(err) => result.errors.push(err),
    }
    result
}

fn clear_java_download_cache(
    jobs: &crate::runtime_jobs::RuntimeJobs,
    root: &Path,
    result: &mut CacheClearResult,
) {
    let path = crate::java::download_cache_dir(root);
    let cache_lock = jobs.download_lock(path.clone());
    let Ok(_guard) = cache_lock.try_lock() else {
        result.java_download_in_use = true;
        return;
    };
    if let Err(err) = remove_download_directory(&path, &mut result.freed) {
        result.errors.push(format!("{}: {err}", path.display()));
    }
}

fn remove_download_directory(path: &Path, freed: &mut u64) -> std::io::Result<()> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if !meta.is_dir() || crate::fsutil::is_link_or_reparse_point(&meta) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid cache directory",
        ));
    }
    let mut first_error = None;
    for entry in std::fs::read_dir(path)? {
        let removal = entry.and_then(|entry| {
            let info = entry.metadata()?;
            let child = entry.path();
            if info.is_dir() && !crate::fsutil::is_link_or_reparse_point(&info) {
                remove_download_directory(&child, freed)
            } else {
                *freed += remove_cache_file(&child)?;
                Ok(())
            }
        });
        if let Err(err) = removal {
            first_error.get_or_insert(err);
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => match std::fs::remove_dir(path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        },
    }
}

fn remove_cache_file(path: &Path) -> std::io::Result<u64> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let size = if meta.is_file() && !crate::fsutil::is_link_or_reparse_point(&meta) {
        meta.len()
    } else {
        0
    };
    std::fs::remove_file(path)?;
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_uses_instance_totals_and_keeps_top_level_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("instance")).unwrap();
        let metadata = br#"{"name":"Launch test"}"#;
        std::fs::write(root.path().join("instance/qookix.json"), metadata).unwrap();
        std::fs::write(root.path().join("instance/options.txt"), b"abc").unwrap();
        std::fs::write(root.path().join("root.txt"), b"abcd").unwrap();
        let (category, instances) =
            scan_instances(root.path(), "qookix.json", "instances", "Instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].name, "Launch test");
        assert_eq!(instances[0].size, metadata.len() as u64 + 3);
        assert_eq!(category.size, instances[0].size + 4);
        assert_eq!(category.files, 3);
    }

    #[test]
    fn downloads_in_use_are_skipped_and_only_successful_removals_count() {
        let root = tempfile::tempdir().unwrap();
        let path = crate::java::download_cache_dir(root.path());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("java.zip"), b"archive").unwrap();
        let jobs = crate::runtime_jobs::RuntimeJobs::default();
        let lock = jobs.download_lock(path.clone());
        let guard = lock.try_lock().unwrap();
        let mut result = CacheClearResult {
            freed: 0,
            java_download_in_use: false,
            errors: Vec::new(),
        };
        clear_java_download_cache(&jobs, root.path(), &mut result);
        assert!(result.java_download_in_use);
        assert_eq!(result.freed, 0);
        assert!(path.join("java.zip").is_file());
        drop(guard);
        assert!(remove_cache_file(&path).is_err());
        clear_java_download_cache(&jobs, root.path(), &mut result);
        assert_eq!(result.freed, 7);
        assert!(result.errors.is_empty());
        assert!(!path.exists());
    }
}
