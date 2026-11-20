//! 世界存档备份：把 `saves/<world>` 打包为 zip 快照，支持恢复与删除。
//! 备份存放于 `instances/<id>/backups/<world>/`。

use crate::state::AppState;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// 单个备份快照的元信息
#[derive(Serialize, Clone, Debug)]
pub struct BackupInfo {
    pub filename: String,
    pub size: u64,
    /// unix 秒
    pub modified: u64,
}

fn backups_dir(state: &AppState, instance_id: &str, world: &str) -> PathBuf {
    state
        .instances_dir()
        .join(instance_id)
        .join("backups")
        .join(world)
}

fn world_dir(state: &AppState, instance_id: &str, world: &str) -> PathBuf {
    state.instances_dir().join(instance_id).join("saves").join(world)
}

fn backup_filename(world: &str) -> String {
    format!("{world}-{}-{}.zip", crate::util::now_secs(), uuid::Uuid::new_v4().simple())
}

fn info_from_file(p: &Path) -> Option<BackupInfo> {
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(BackupInfo {
        filename: p.file_name()?.to_string_lossy().to_string(),
        size: meta.len(),
        modified,
    })
}

/// 列出某个世界的全部备份（按修改时间倒序）
pub fn list_backups(state: &AppState, instance_id: &str, world: &str) -> Vec<BackupInfo> {
    let dir = backups_dir(state, instance_id, world);
    let mut out: Vec<BackupInfo> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "zip").unwrap_or(false))
                .filter_map(|p| info_from_file(&p))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// 递归把 `dir` 下的文件写入 zip（zip 内路径相对 `dir`）
fn write_dir_to_zip(
    zw: &mut zip::ZipWriter<&mut std::fs::File>,
    dir: &Path,
    prefix: &str,
    ancestors: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let canonical = dir.canonicalize().map_err(|e| format!("解析存档目录失败: {e}"))?;
    if ancestors.contains(&canonical) {
        return Err(format!("存档目录包含循环链接: {}", dir.display()));
    }
    ancestors.push(canonical);
    let rd = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    for e in rd {
        let e = e.map_err(|e| format!("读取存档条目失败: {e}"))?;
        let p = e.path();
        let name = p
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("无法读取存档文件名: {}", p.display()))?;
        if !crate::fsutil::is_portable_filename(name) {
            return Err(format!("备份不支持该存档条目名称: {}", p.display()));
        }
        let rel = if prefix.is_empty() { name.to_owned() } else { format!("{prefix}/{name}") };
        let metadata = std::fs::metadata(&p)
            .map_err(|e| format!("读取 {} 的元数据失败: {e}", p.display()))?;
        if metadata.is_dir() {
            write_dir_to_zip(zw, &p, &rel, ancestors)?;
        } else if metadata.is_file() {
            let mut f = std::fs::File::open(&p).map_err(|e| format!("打开 {} 失败: {e}", p.display()))?;
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(metadata.len() >= u64::from(u32::MAX));
            zw.start_file(rel.clone(), opts)
                .map_err(|e| format!("写入 zip 条目 {rel} 失败: {e}"))?;
            std::io::copy(&mut f, zw).map_err(|e| format!("写入 {rel} 内容失败: {e}"))?;
        }
    }
    ancestors.pop();
    Ok(())
}

fn archive_world(src: &Path, backups: &Path, world: &str) -> Result<BackupInfo, String> {
    std::fs::create_dir_all(backups).map_err(|e| format!("创建备份目录失败: {e}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(backups)
        .map_err(|e| format!("创建临时备份文件失败: {e}"))?;
    let mut zip = zip::ZipWriter::new(temp.as_file_mut());
    write_dir_to_zip(&mut zip, src, "", &mut Vec::new())?;
    let file = zip.finish().map_err(|e| format!("完成 zip 失败: {e}"))?;
    file.sync_all().map_err(|e| format!("保存备份文件失败: {e}"))?;
    let path = backups.join(backup_filename(world));
    temp.persist_noclobber(&path).map_err(|e| format!("发布备份失败: {e}"))?;
    info_from_file(&path).ok_or_else(|| "读取备份信息失败".into())
}

/// 创建备份：把 `saves/<world>` 打包为 zip。返回备份信息。
pub fn create_backup(state: &AppState, instance_id: &str, world: &str) -> Result<BackupInfo, String> {
    let _files = state.jobs.reserve_game_files(instance_id)?;
    if !crate::fsutil::is_safe_filename(world) {
        return Err("非法的世界目录名".into());
    }
    let src = world_dir(state, instance_id, world);
    if !src.is_dir() {
        return Err(format!("世界目录不存在: {}", src.display()));
    }
    archive_world(&src, &backups_dir(state, instance_id, world), world)
}

/// 校验并暂存快照后，再替换当前世界。
pub fn restore_backup(state: &AppState, instance_id: &str, world: &str, filename: &str) -> Result<(), String> {
    let _files = state.jobs.reserve_game_files(instance_id)?;
    if !crate::fsutil::is_safe_filename(world) || !crate::fsutil::is_safe_filename(filename) {
        return Err("非法的路径参数".into());
    }
    let backups = backups_dir(state, instance_id, world);
    let zip_path = backups.join(filename);
    if !zip_path.is_file() {
        return Err("备份文件不存在".into());
    }
    restore_world(&zip_path, &world_dir(state, instance_id, world), &backups, world)
}

fn restore_world(zip_path: &Path, dest: &Path, backups: &Path, world: &str) -> Result<(), String> {
    let saves = dest.parent().ok_or("无法确定存档父目录")?;
    std::fs::create_dir_all(saves).map_err(|e| format!("创建存档目录失败: {e}"))?;
    let staging = tempfile::Builder::new().prefix(".qookix-restore-").tempdir_in(saves)
        .map_err(|e| format!("创建恢复暂存目录失败: {e}"))?;
    let staged_world = staging.path().join("world");
    if crate::archive::extract_zip(zip_path, &staged_world, &[])? == 0 {
        return Err("备份内容为空".into());
    }
    validate_staged_world(&staged_world)?;

    match std::fs::symlink_metadata(dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::rename(&staged_world, dest).map_err(|e| format!("提交恢复存档失败: {e}"))?;
            crate::nbt::map::invalidate_world(dest);
            return Ok(());
        }
        Err(e) => return Err(format!("检查现有存档失败: {e}")),
        Ok(_) if dest.is_dir() => {}
        Ok(_) => return Err("现有存档路径不是可读取的目录".into()),
    }

    archive_world(dest, backups, world).map_err(|e| format!("恢复前创建安全备份失败: {e}"))?;
    replace_world(&staged_world, dest, saves)
}

pub(crate) fn validate_staged_world(staged_world: &Path) -> Result<(), String> {
    let root = crate::nbt::level::read_level(&staged_world.join("level.dat"))
        .map_err(|e| format!("备份中的 level.dat 无效: {e}"))?;
    use crate::nbt::level::{get, world_data};
    let data = world_data(&root);
    if !["LevelName", "RandomSeed", "WorldGenSettings", "SpawnX"].iter().any(|key| get(data, key).is_some()) {
        return Err("备份中的 level.dat 缺少世界数据".into());
    }

    Ok(())
}

pub(crate) fn commit_staged_world(staged: &Path, destination: &Path, backup: &Path) -> Result<(), String> {
    std::fs::rename(destination, backup).map_err(|error| format!("暂存现有存档失败: {error}"))?;
    if let Err(error) = std::fs::rename(staged, destination) {
        if let Err(rollback) = std::fs::rename(backup, destination) {
            return Err(format!("提交恢复失败: {error}。回滚失败: {rollback}。原存档保留在 {}", backup.display()));
        }
        return Err(format!("提交恢复失败，原存档已恢复: {error}"));
    }
    crate::nbt::map::invalidate_world(destination);
    Ok(())
}

fn replace_world(staged_world: &Path, dest: &Path, saves: &Path) -> Result<(), String> {
    // 保留原目录，直到新存档完成切换。异常退出时也保留这个回滚副本。
    let recovery = tempfile::Builder::new().prefix(".qookix-previous-").tempdir_in(saves)
        .map_err(|e| format!("创建回滚目录失败: {e}"))?;
    let previous = recovery.path().join("original");
    let recovery = recovery.keep();
    let result = commit_staged_world(staged_world, dest, &previous);
    if result.is_ok() || !previous.exists() {
        crate::fsutil::fs_best_effort("remove_dir_all", &recovery, std::fs::remove_dir_all(&recovery));
    }
    result
}

/// 删除备份
pub fn delete_backup(state: &AppState, instance_id: &str, world: &str, filename: &str) -> Result<(), String> {
    if !crate::fsutil::is_safe_filename(world) || !crate::fsutil::is_safe_filename(filename) {
        return Err("非法的路径参数".into());
    }
    let p = backups_dir(state, instance_id, world).join(filename);
    if !p.is_file() {
        return Err("备份文件不存在".into());
    }
    std::fs::remove_file(&p).map_err(|e| format!("删除失败: {e}"))
}

/// 读取 zip 中的单个条目（恢复流程之外的辅助）
#[allow(dead_code)]
fn read_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>, String> {
    let mut e = archive
        .by_name(name)
        .map_err(|e| format!("zip 内缺少 {name}: {e}"))?;
    let mut buf = Vec::new();
    Read::read_to_end(&mut e, &mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::Value;
    use std::fs;

    fn write_world(path: &Path, name: &str) {
        fs::create_dir_all(path.join("region")).unwrap();
        fs::write(path.join("region/r.0.0.mca"), name).unwrap();
        let data = Value::Compound([("LevelName".into(), Value::String(name.into()))].into());
        let level = Value::Compound([("Data".into(), data)].into());
        crate::nbt::level::write_level(&path.join("level.dat"), &level).unwrap();
    }

    #[test]
    fn restores_valid_zip_with_separate_safety_backup() {
        let root = tempfile::tempdir().unwrap();
        let sample = root.path().join("sample");
        let saves = root.path().join("saves");
        let current = saves.join("world");
        let backups = root.path().join("backups");
        write_world(&sample, "snapshot");
        write_world(&current, "current");

        let snapshot = archive_world(&sample, &backups, "world").unwrap();
        let snapshot_path = backups.join(&snapshot.filename);
        restore_world(&snapshot_path, &current, &backups, "world").unwrap();

        assert_eq!(fs::read(current.join("region/r.0.0.mca")).unwrap(), b"snapshot");
        assert_eq!(fs::read_dir(saves).unwrap().count(), 1);
        let paths = fs::read_dir(&backups).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        let safety = paths.iter().find(|p| **p != snapshot_path).unwrap();
        assert_eq!(crate::archive::read_zip_entry(safety, "region/r.0.0.mca").unwrap(), b"current");
    }

    #[test]
    fn leaves_current_world_untouched_if_snapshot_or_safety_backup_fails() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("saves/world");
        let sample = root.path().join("sample");
        let backups = root.path().join("backups");
        write_world(&current, "current");

        fs::create_dir(&sample).unwrap();
        fs::write(sample.join("note.txt"), "not a world").unwrap();
        let incomplete = archive_world(&sample, &backups, "world").unwrap();
        assert!(restore_world(&backups.join(incomplete.filename), &current, &backups, "world").is_err());

        let invalid_zip = root.path().join("corrupt.zip");
        fs::write(&invalid_zip, b"incomplete").unwrap();
        assert!(restore_world(&invalid_zip, &current, &backups, "world").is_err());

        write_world(&sample, "snapshot");
        let valid = archive_world(&sample, &backups, "world").unwrap();
        let unavailable = root.path().join("not-a-directory");
        fs::write(&unavailable, "blocks backup directory").unwrap();
        let error = restore_world(&backups.join(valid.filename), &current, &unavailable, "world").unwrap_err();
        assert!(error.contains("恢复前创建安全备份失败"));
        assert_eq!(fs::read(current.join("region/r.0.0.mca")).unwrap(), b"current");
        assert_eq!(fs::read_dir(current.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn moves_original_back_if_commit_fails() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("world");
        write_world(&current, "current");

        assert!(replace_world(&root.path().join("missing-stage"), &current, root.path()).is_err());
        assert_eq!(fs::read(current.join("region/r.0.0.mca")).unwrap(), b"current");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
