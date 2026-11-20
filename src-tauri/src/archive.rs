//! ZIP 和 JAR 归档读取、校验与解压。

use crate::fsutil::{is_link_or_reparse_point, is_portable_filename};
use std::io::Read;
use std::path::Path;

/// 已存在的库文件是否可当作 zip/jar 打开（结构完整）。
///
/// 安装期用它决定「跳过」还是「重新下载」：之前只看文件是否存在，被截断/写坏的
/// jar 永远修不好，「重新安装游戏」也没用。注意**不能**拿 json 里的 sha1/size 去
/// 比对已存在的文件——Forge/NeoForge 的 processor 产物就在同一路径下，内容与
/// maven 原件不同，比对会把它们误当成损坏并覆盖掉。
pub fn zip_readable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    // 空 zip 的 EOCD 也有 22 字节，比这还小必然不是有效归档
    if meta.len() < 22 {
        return false;
    }
    std::fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some()
}

/// Read the class-file major version of the first `.class` entry in a jar.
/// Java major = class_major - 44 (Java 8 = 52, 17 = 61, 21 = 65, 25 = 69).
pub fn jar_class_version(path: &Path) -> Option<u32> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for i in 0..archive.len() {
        let name = archive.by_index(i).ok()?.name().to_string();
        if !name.ends_with(".class") {
            continue;
        }
        let mut entry = archive.by_index(i).ok()?;
        let mut buf = [0u8; 8];
        std::io::Read::read_exact(&mut entry, &mut buf).ok()?;
        // CAFEBABE + minor(2) + major(2)
        if buf[0] == 0xCA && buf[1] == 0xFE && buf[2] == 0xBA && buf[3] == 0xBE {
            return Some(u16::from_be_bytes([buf[6], buf[7]]) as u32);
        }
    }
    None
}

/// Extract a zip archive into `dest`, skipping entries whose path starts with
/// any of `skip_prefixes`. Returns the number of files written.
pub fn extract_zip(path: &Path, dest: &Path, skip_prefixes: &[&str]) -> Result<usize, String> {
    extract_zip_inner(path, dest, skip_prefixes, None)
}

/// Like `extract_zip`, but also strips `strip` from each entry's path
/// (used to unpack an `overrides/` folder to the archive root).
pub fn extract_zip_strip(
    path: &Path,
    dest: &Path,
    strip: &str,
    skip_prefixes: &[&str],
) -> Result<usize, String> {
    extract_zip_inner(path, dest, skip_prefixes, Some(strip))
}

fn extract_zip_inner(
    path: &Path,
    dest: &Path,
    skip_prefixes: &[&str],
    strip: Option<&str>,
) -> Result<usize, String> {
    extract_with_staging(dest, |staged| {
        unpack_into_stage(path, staged, skip_prefixes, strip)
    })
}

pub(crate) fn unpack_into_stage(
    path: &Path,
    stage: &Path,
    skip_prefixes: &[&str],
    strip: Option<&str>,
) -> Result<usize, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("打开 {} 失败: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {e}"))?;
    extract_zip_archive(&mut archive, stage, skip_prefixes, strip)
}

pub(crate) fn extract_with_staging(
    dest: &Path,
    unpack: impl FnOnce(&Path) -> Result<usize, String>,
) -> Result<usize, String> {
    let parent = dest.parent().ok_or("无法确定解压目标的父目录")?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建解压父目录失败: {error}"))?;
    let temp = tempfile::Builder::new()
        .prefix(".qookix-extract-")
        .tempdir_in(parent)
        .map_err(|error| format!("创建解压暂存目录失败: {error}"))?;
    let stage = temp.path().join("content");
    let count = unpack(&stage)?;
    match std::fs::symlink_metadata(dest) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::rename(&stage, dest).map_err(|error| format!("提交解压目录失败: {error}"))?;
        }
        Err(error) => return Err(format!("检查解压目录失败: {error}")),
        Ok(_) => {
            ensure_zip_dir(dest)?;
            let root = dest
                .canonicalize()
                .map_err(|error| format!("解析解压目录失败: {error}"))?;
            check_staged_tree(&stage, &root, false)?;
            check_staged_tree(&stage, &root, true)?;
        }
    }
    Ok(count)
}

/// A staging tree contains only regular files and directories created by the ZIP reader.
/// Validate the complete destination tree before publishing any of its entries.
fn check_staged_tree(source: &Path, destination: &Path, publish: bool) -> Result<(), String> {
    for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        match std::fs::symlink_metadata(&target) {
            Ok(meta)
                if is_link_or_reparse_point(&meta)
                    || (kind.is_dir() && !meta.is_dir())
                    || (kind.is_file() && !meta.is_file()) =>
            {
                return Err(format!(
                    "解压目标包含链接或不匹配的文件类型: {}",
                    target.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("检查解压路径失败: {error}")),
        }
        if kind.is_dir() {
            if publish {
                ensure_zip_dir(&target)?;
            }
            check_staged_tree(&entry.path(), &target, publish)?;
        } else if kind.is_file() {
            if publish {
                std::fs::rename(entry.path(), &target)
                    .map_err(|error| format!("提交解压文件 {} 失败: {error}", target.display()))?;
            }
        } else {
            return Err("解压暂存目录包含非普通文件".into());
        }
    }
    Ok(())
}

fn ensure_zip_dir(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !is_link_or_reparse_point(&meta) => Ok(()),
        Ok(_) => Err(format!("ZIP 目标不是普通目录: {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map_err(|e| format!("创建目录 {} 失败: {e}", path.display()))
        }
        Err(e) => Err(format!("检查目录 {} 失败: {e}", path.display())),
    }
}

pub(crate) fn extract_zip_archive<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dest: &Path,
    skip_prefixes: &[&str],
    strip: Option<&str>,
) -> Result<usize, String> {
    extract_zip_mapped(archive, dest, |name, is_dir| {
        if skip_prefixes.iter().any(|prefix| name.starts_with(prefix)) {
            return Ok(None);
        }
        let rel = match strip {
            Some(prefix) => match name.strip_prefix(prefix) {
                Some(rel) => rel,
                None => return Ok(None),
            },
            None => name,
        };
        if rel.is_empty() && strip.is_some() && is_dir {
            return Ok(None);
        }
        Ok(Some(rel.to_owned()))
    })
}

pub(crate) fn extract_zip_mapped<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dest: &Path,
    map_path: impl FnMut(&str, bool) -> Result<Option<String>, String>,
) -> Result<usize, String> {
    extract_zip_mapped_with_limits(
        archive,
        dest,
        ZipLimits { entries: 250_000, expanded_bytes: 128 * 1024 * 1024 * 1024 },
        map_path,
    )
}

pub(crate) struct ZipLimits {
    pub entries: usize,
    pub expanded_bytes: u64,
}

pub(crate) fn extract_zip_mapped_with_limits<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dest: &Path,
    limits: ZipLimits,
    mut map_path: impl FnMut(&str, bool) -> Result<Option<String>, String>,
) -> Result<usize, String> {
    if archive.len() > limits.entries {
        return Err("ZIP 条目数超过限制".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("创建解压目录失败: {e}"))?;
    ensure_zip_dir(dest)?;
    let root = dest
        .canonicalize()
        .map_err(|e| format!("解析解压目录失败: {e}"))?;
    let mut count = 0usize;
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        let is_dir = entry.is_dir();
        let Some(relative) = map_path(&name, is_dir)? else {
            continue;
        };
        let rel = relative.as_str();
        let kind = entry.unix_mode().unwrap_or_default() & 0o170000;
        if entry.is_symlink() || !matches!(kind, 0 | 0o100000 | 0o040000) {
            return Err(format!("ZIP 包含链接或特殊文件: {name}"));
        }
        let rel = if is_dir {
            rel.strip_suffix('/').unwrap_or(rel)
        } else {
            rel
        };
        let mut parts = rel.split('/').peekable();
        if !parts.clone().all(is_portable_filename) {
            return Err(format!("ZIP 条目路径无效: {name}"));
        }
        let mut out = root.clone();
        while let Some(part) = parts.next() {
            out.push(part);
            if is_dir || parts.peek().is_some() {
                ensure_zip_dir(&out)?;
            }
        }
        if is_dir {
            continue;
        }
        let size = entry.size();
        expanded = expanded
            .checked_add(size)
            .filter(|total| *total <= limits.expanded_bytes)
            .ok_or("ZIP 展开大小超过限制")?;
        match std::fs::symlink_metadata(&out) {
            Ok(meta) if meta.is_file() && !is_link_or_reparse_point(&meta) => {}
            Ok(_) => return Err(format!("ZIP 目标不是普通文件: {}", out.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("检查文件 {} 失败: {e}", out.display())),
        }
        let parent = out.parent().ok_or("无法确定 ZIP 条目的父目录")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("创建解压临时文件失败: {e}"))?;
        let bytes = std::io::copy(&mut entry.by_ref().take(size + 1), temp.as_file_mut())
            .map_err(|e| format!("解压 {name} 失败: {e}"))?;
        if bytes != size {
            return Err(format!("ZIP 条目大小与记录不符: {name}"));
        }
        temp.persist(&out)
            .map_err(|e| format!("提交解压文件 {} 失败: {e}", out.display()))?;
        count += 1;
    }
    Ok(count)
}

/// Read a single entry from a zip archive as bytes.
pub fn read_zip_entry(path: &Path, entry_name: &str) -> Result<Vec<u8>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("打开 {} 失败: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析 zip 失败: {e}"))?;
    let mut entry = archive
        .by_name(entry_name)
        .map_err(|e| format!("zip 内缺少 {entry_name}: {e}"))?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_late_entry_leaves_existing_extraction_target_untouched() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("fixture.zip");
        let dest = root.path().join("target");
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("existing.txt"), "old").unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("existing.txt", options).unwrap();
        zip.write_all(b"replacement").unwrap();
        zip.start_file("nested/../invalid.txt", options).unwrap();
        zip.write_all(b"invalid").unwrap();
        zip.finish().unwrap();

        assert!(extract_zip(&archive, &dest, &[]).is_err());
        assert_eq!(
            std::fs::read_to_string(dest.join("existing.txt")).unwrap(),
            "old"
        );
        assert_eq!(std::fs::read_dir(dest).unwrap().count(), 1);
    }

    #[test]
    fn zip_extracts_only_overrides_when_stripping_the_prefix() {
        use std::io::{Cursor, Write};

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.add_directory("overrides/", options).unwrap();
        writer
            .start_file("overrides/config/.settings", options)
            .unwrap();
        writer.write_all(b"override").unwrap();
        writer.start_file("config/.settings", options).unwrap();
        writer.write_all(b"base").unwrap();
        writer.start_file("pack.png", options).unwrap();
        writer.write_all(b"image").unwrap();
        let mut archive = zip::ZipArchive::new(writer.finish().unwrap()).unwrap();
        let dest = tempfile::tempdir().unwrap();

        let extracted =
            extract_zip_archive(&mut archive, dest.path(), &[], Some("overrides/")).unwrap();

        assert_eq!(extracted, 1);
        assert_eq!(
            std::fs::read(dest.path().join("config/.settings")).unwrap(),
            b"override"
        );
        assert!(!dest.path().join("pack.png").exists());
    }
}
