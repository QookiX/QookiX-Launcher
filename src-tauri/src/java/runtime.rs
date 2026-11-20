use super::{clean_path, find_runtime_bins, invalidate_java_cache, probe_java, ScanBudget};
use crate::download::{download_many, DownloadItem};
use crate::install::TaskCtx;
use crate::models::{Instance, JavaInfo};
use crate::state::AppState;
use serde::Deserialize;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const INSTALL_GUIDE: &str = "https://adoptium.net/installation/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    fn suffix(self) -> &'static str {
        match self {
            Self::Zip => ".zip",
            Self::TarGz => ".tar.gz",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Platform {
    os: &'static str,
    arch: &'static str,
    format: ArchiveFormat,
}

impl Platform {
    fn find(os: &str, arch: &str) -> Option<Self> {
        let (os, format) = match os {
            "windows" => ("windows", ArchiveFormat::Zip),
            "linux" => ("linux", ArchiveFormat::TarGz),
            "macos" => ("mac", ArchiveFormat::TarGz),
            _ => return None,
        };
        let arch = match arch {
            "x86_64" => "x64",
            "aarch64" => "aarch64",
            "arm" if os == "linux" => "arm",
            "x86" if os == "windows" => "x86",
            _ => return None,
        };
        Some(Self { os, arch, format })
    }

    fn assets_url(self, major: u32) -> String {
        format!(
            "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?os={}&architecture={}&image_type=jre&vendor=eclipse",
            self.os, self.arch
        )
    }
}

#[derive(Deserialize)]
struct Asset {
    binary: Binary,
}

#[derive(Deserialize)]
struct Binary {
    package: Package,
}

#[derive(Deserialize)]
struct Package {
    link: String,
    name: String,
    size: u64,
    checksum: String,
}

impl Package {
    fn matches(&self, platform: Platform) -> bool {
        crate::fsutil::is_portable_filename(&self.name)
            && self.name.ends_with(platform.format.suffix())
            && self.size > 0
            && self.size <= 1024 * 1024 * 1024
            && self.checksum.len() == 64
            && self.checksum.bytes().all(|b| b.is_ascii_hexdigit())
            && reqwest::Url::parse(&self.link)
                .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
    }

    fn valid_file(&self, path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|meta| {
            meta.is_file()
                && !crate::fsutil::is_link_or_reparse_point(&meta)
                && meta.len() == self.size
        }) && crate::util::file_sha256(path)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(&self.checksum))
    }
}

pub(crate) fn download_cache_dir(root: &Path) -> PathBuf {
    root.join("runtimes").join("downloads")
}

/// Install only a platform-matched and verified runtime.
pub async fn download_java_runtime(
    app: tauri::AppHandle,
    state: &AppState,
    major: u32,
) -> Result<JavaInfo, String> {
    let platform = Platform::find(std::env::consts::OS, std::env::consts::ARCH)
        .ok_or_else(|| format!("当前系统架构不支持自动补装 Java。请手动安装 Java {major}，并在启动器设置中选择它：{INSTALL_GUIDE}"))?;
    let cache = download_cache_dir(&state.root);
    let cache_guard = state.jobs.download_lock(cache.clone()).lock_owned().await;
    let dest = state
        .root
        .join("runtimes")
        .join("java")
        .join(major.to_string());
    let task_id = state.next_task_id();
    let lease = state
        .jobs
        .register_install(task_id, "@java-runtime", None)?;
    let cancel = lease.cancel.clone();
    let instance = Instance {
        id: "@java-runtime".into(),
        name: "Java 运行时".into(),
        ..Default::default()
    };
    let source = format!("Java 运行时 {major}");
    let mut task = TaskCtx::new(&app, &instance, &source, task_id);
    task.emit("runtime", &format!("正在准备 Java {major}…"), 0, 1);
    let asset_url = platform.assets_url(major);

    let result: Result<JavaInfo, String> = async {
        let assets: Vec<Asset> = tokio::select! {
            _ = cancel.cancelled() => return Err("Java 安装已取消".into()),
            assets = crate::download::get_json(&state.client, &asset_url) =>
                assets.map_err(|err| format!("无法获取 Java {major} 的安装包：{err}。也可手动安装并在设置中选择 Java：{INSTALL_GUIDE}"))?,
        };
        let package = assets.into_iter().map(|asset| asset.binary.package)
            .find(|package| package.matches(platform))
            .ok_or_else(|| format!("Temurin 未提供适用于 {} {} 的 Java {major} JRE。请手动安装并在设置中选择 Java：{INSTALL_GUIDE}", platform.os, platform.arch))?;
        tokio::fs::create_dir_all(&cache).await.map_err(|err| err.to_string())?;
        let archive = cache.join(&package.name);
        let mut urls = Vec::new();
        if platform.os == "windows" && platform.arch == "x64"
            && package.link.starts_with("https://github.com/adoptium/temurin")
        {
            urls.push(format!("https://mirrors.tuna.tsinghua.edu.cn/Adoptium/{major}/jre/x64/windows/{}", package.name));
        }
        urls.push(package.link.clone());
        if !package.valid_file(&archive) {
            remove_invalid_archive(&archive).await?;
            let mut last_error = String::new();
            for url in urls {
                let item = DownloadItem { url, dest: archive.clone(), sha1: None, sha512: None,
                    size: Some(package.size), label: package.name.clone() };
                match download_many(app.clone(), state, task_id, "runtime", vec![item]).await {
                    Ok(_) if package.valid_file(&archive) => {
                        last_error.clear();
                        break;
                    }
                    Ok(_) => {
                        last_error = "Java 安装包 SHA-256 校验失败".into();
                        remove_invalid_archive(&archive).await?;
                    }
                    Err(err) => last_error = err,
                }
                if cancel.is_cancelled() {
                    return Err("Java 安装已取消".into());
                }
            }
            if !last_error.is_empty() {
                return Err(last_error);
            }
        }
        task.emit("runtime", &format!("正在验证并解压 Java {major}…"), 0, 1);
        let java = tokio::task::spawn_blocking(move || {
            let _guards = (lease, cache_guard);
            install_archive(&archive, &dest, platform.format, major, &cancel)
        })
        .await
        .map_err(|err| err.to_string())??;
        *state.java_cache.lock().unwrap() = None;
        invalidate_java_cache(&state.root);
        Ok(java)
    }.await;
    match &result {
        Ok(_) => task.finish_ok("Java 运行时就绪"),
        Err(err) => task.finish_err(err),
    }
    result
}

async fn remove_invalid_archive(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("无法清理损坏的 Java 安装包：{error}")),
    }
}

fn install_archive(
    archive: &Path,
    dest: &Path,
    format: ArchiveFormat,
    major: u32,
    cancel: &CancellationToken,
) -> Result<JavaInfo, String> {
    let parent = dest.parent().ok_or("Java 安装目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    match format {
        ArchiveFormat::Zip => {
            crate::archive::extract_zip(archive, stage.path(), &["META-INF/"])?;
        }
        ArchiveFormat::TarGz => unpack_tar(archive, stage.path())?,
    }
    let mut budget = ScanBudget {
        deadline: Instant::now() + Duration::from_secs(3),
        remaining_entries: 8192,
    };
    let mut candidates = find_runtime_bins(stage.path(), &mut budget).into_iter();
    let java = candidates.find_map(|bin| {
        let info = probe_java(&bin)?;
        (info.major == major).then_some((bin, info))
    });
    let (bin, mut info) = java.ok_or_else(|| format!("解压后没有找到可运行的 Java {major}"))?;
    let relative = bin.strip_prefix(stage.path()).map_err(|e| e.to_string())?;
    info.path = clean_path(&dest.join(relative));
    if cancel.is_cancelled() {
        return Err("Java 安装已取消".into());
    }
    let backup = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    let previous = backup.path().join("previous");
    let moved = match fs::rename(dest, &previous) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => return Err(format!("无法暂存原 Java 运行时：{err}")),
    };
    if let Err(err) = fs::rename(stage.path(), dest) {
        if moved {
            if let Err(restore) = fs::rename(&previous, dest) {
                let saved = backup.keep();
                return Err(format!(
                    "无法安装 Java：{err}。原运行时保留在 {}，自动恢复失败：{restore}",
                    saved.join("previous").display()
                ));
            }
        }
        return Err(format!("无法安装 Java：{err}"));
    }
    Ok(info)
}

fn unpack_tar(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut links = Vec::new();
    let mut expanded = 0u64;
    for (index, entry) in archive.entries().map_err(|e| e.to_string())?.enumerate() {
        let mut entry = entry.map_err(|e| e.to_string())?;
        expanded = expanded
            .checked_add(entry.size())
            .ok_or("Java 安装包展开大小无效")?;
        if index >= 50_000 || expanded > 2 * 1024 * 1024 * 1024 {
            return Err("Java 安装包展开大小或条目数超过限制".into());
        }
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let mut depth = 0;
        for component in path.components() {
            match component {
                Component::Normal(_) => depth += 1,
                Component::CurDir => {}
                _ => return Err("Java 安装包包含越界路径".into()),
            }
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let target = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or("归档链接缺少目标")?;
            let base_depth = if kind.is_symlink() && depth > 0 {
                depth - 1
            } else {
                0
            };
            if !link_stays_inside(&target, base_depth) {
                return Err("Java 安装包包含越界链接".into());
            }
            links.push(path.clone());
        } else if !kind.is_file() && !kind.is_dir() {
            return Err("Java 安装包包含特殊文件".into());
        }
        if depth == 0 && kind.is_dir() {
            continue;
        }
        if !entry
            .unpack_in(dest)
            .map_err(|err| format!("无法解压 {}：{err}", path.display()))?
        {
            return Err(format!("Java 安装包包含无效路径：{}", path.display()));
        }
    }
    let canonical_dest = dest.canonicalize().map_err(|e| e.to_string())?;
    for link in links {
        let target = dest
            .join(&link)
            .canonicalize()
            .map_err(|e| format!("归档链接 {} 无效：{e}", link.display()))?;
        if !target.starts_with(&canonical_dest) {
            return Err("Java 安装包包含越界链接".into());
        }
    }
    Ok(())
}

fn link_stays_inside(target: &Path, mut depth: usize) -> bool {
    !target.as_os_str().is_empty()
        && target.components().all(|component| match component {
            Component::Normal(_) => {
                depth += 1;
                true
            }
            Component::CurDir => true,
            Component::ParentDir if depth > 0 => {
                depth -= 1;
                true
            }
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms_choose_native_adoptium_archives() {
        let windows = Platform::find("windows", "x86_64").unwrap();
        assert_eq!(windows.format, ArchiveFormat::Zip);
        assert!(windows
            .assets_url(17)
            .contains("os=windows&architecture=x64"));
        let mac = Platform::find("macos", "aarch64").unwrap();
        assert_eq!(mac.format, ArchiveFormat::TarGz);
        assert!(mac.assets_url(21).contains("os=mac&architecture=aarch64"));
        assert_eq!(
            Platform::find("linux", "x86_64").unwrap().format,
            ArchiveFormat::TarGz
        );
        assert!(Platform::find("android", "aarch64").is_none());
    }

    #[test]
    fn package_requires_safe_name_native_format_and_sha256() {
        let platform = Platform::find("windows", "x86_64").unwrap();
        let mut package = Package {
            name: "java.zip".into(),
            link: "https://example.invalid/java.zip".into(),
            size: 123,
            checksum: "a".repeat(64),
        };
        assert!(package.matches(platform));
        package.name = "old/../java.zip".into();
        assert!(!package.matches(platform));
        package.name = "java.tar.gz".into();
        assert!(!package.matches(platform));
        package.name = "java.zip".into();
        package.checksum.clear();
        assert!(!package.matches(platform));
    }

    #[test]
    fn tar_link_targets_cannot_escape_the_installation_stage() {
        assert!(link_stays_inside(Path::new("../lib/java"), 2));
        assert!(!link_stays_inside(Path::new("../../../outside"), 2));
        assert!(!link_stays_inside(Path::new("/absolute"), 2));
    }
}
