use crate::models::{InstalledContent, Instance};
use crate::state::AppState;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(crate) struct Replacement {
    previous: InstalledContent,
    original: PathBuf,
    stamp: (u64, SystemTime),
    stage: tempfile::TempDir,
    _files: crate::runtime_jobs::ProcessLease,
}

fn file_stamp(path: &Path) -> io::Result<(u64, SystemTime)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || crate::fsutil::is_link_or_reparse_point(&metadata) {
        return Err(io::Error::other("内容文件不是普通文件"));
    }
    Ok((metadata.len(), metadata.modified()?))
}

impl Replacement {
    pub(crate) fn prepare(
        state: &AppState,
        instance: &Instance,
        kind: &str,
        filename: &str,
        provider: &str,
        project_id: &str,
    ) -> Result<Self, String> {
        if !crate::fsutil::is_safe_filename(filename) {
            return Err("非法内容文件名".into());
        }
        let files = state.jobs.reserve_game_files(&instance.id)?;
        let previous = crate::instances::into_content_records(instance.clone(), kind)
            .into_iter()
            .find(|record| {
                record.filename == filename
                    && record.source == provider
                    && record.project_id.as_deref() == Some(project_id)
            })
            .ok_or("已安装内容发生变化，请重新检查更新")?;
        let base = state
            .instances_dir()
            .join(&instance.id)
            .join(crate::modrinth::kind_folder(kind));
        let directory = base.canonicalize().map_err(|error| error.to_string())?;
        let active = directory.join(filename);
        let disabled = directory.join(format!("{filename}.disabled"));
        let (original, stamp) = match file_stamp(&active) {
            Ok(stamp) => (active, stamp),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let stamp = file_stamp(&disabled)
                    .map_err(|error| format!("无法读取旧内容文件: {error}"))?;
                (disabled, stamp)
            }
            Err(error) => return Err(format!("无法读取旧内容文件: {error}")),
        };
        let stage = tempfile::Builder::new()
            .prefix(".qookix-update-")
            .tempdir_in(directory)
            .map_err(|error| format!("无法创建内容更新暂存目录: {error}"))?;
        Ok(Self {
            previous,
            original,
            stamp,
            stage,
            _files: files,
        })
    }

    pub(crate) fn download_path(&self) -> PathBuf {
        self.stage.path().join("download")
    }

    pub(crate) fn matches_project(&self, project_id: &str) -> bool {
        self.previous.project_id.as_deref() == Some(project_id)
    }

    pub(crate) fn commit(
        self,
        state: &AppState,
        instance: &Instance,
        kind: &str,
        mut next: InstalledContent,
    ) -> Result<(), String> {
        next.enabled =
            self.original.file_name() == Some(std::ffi::OsStr::new(&self.previous.filename));
        let target_filename = if next.enabled {
            next.filename.clone()
        } else {
            format!("{}.disabled", next.filename)
        };
        let result = crate::instances::replace_content_revision(
            state,
            &instance.id,
            kind,
            &self.previous,
            next,
            |commit_metadata| {
                let live = self.original.parent().ok_or("内容目录不存在")?;
                let expected_directory = state
                    .instances_dir()
                    .join(&instance.id)
                    .join(crate::modrinth::kind_folder(kind));
                if expected_directory.canonicalize().ok().as_deref() != Some(live) {
                    return Err("下载期间内容目录发生变化，请重新检查更新".into());
                }
                if file_stamp(&self.original).ok() != Some(self.stamp) {
                    return Err("下载期间已安装文件发生变化，请重新检查更新".into());
                }
                replace_file(
                    &self.original,
                    &live.join(target_filename),
                    &self.stage,
                    commit_metadata,
                )
            },
        );
        match result {
            Ok(()) => Ok(()),
            Err(error) if self.stage.path().join("previous").is_file() => {
                let path = self.stage.keep();
                Err(format!("{error}。原始文件保存在 {}", path.display()))
            }
            Err(error) => Err(error),
        }
    }
}

fn replace_file(
    old: &Path,
    target: &Path,
    stage: &tempfile::TempDir,
    commit_metadata: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if target.try_exists().map_err(|error| error.to_string())?
        && target.canonicalize().ok() != old.canonicalize().ok()
    {
        return Err("新版本文件名已被其他文件占用".into());
    }
    let backup = stage.path().join("previous");
    let download = stage.path().join("download");
    fs::rename(old, &backup).map_err(|error| format!("无法暂存旧版本: {error}"))?;
    if let Err(error) = fs::rename(&download, target) {
        fs::rename(&backup, old)
            .map_err(|restore| format!("更新文件失败: {error}。还原失败: {restore}"))?;
        return Err(format!("更新文件失败: {error}"));
    }
    if let Err(error) = commit_metadata() {
        fs::rename(target, &download)
            .map_err(|restore| format!("记录更新失败: {error}。移回新文件失败: {restore}"))?;
        fs::rename(&backup, old)
            .map_err(|restore| format!("记录更新失败: {error}。还原失败: {restore}"))?;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_with_same_filename_publishes_the_download() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        let path = root.path().join("stable.jar");
        fs::write(&path, "old").unwrap();
        fs::write(stage.path().join("download"), "new").unwrap();
        replace_file(&path, &path, &stage, || Ok(())).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(stage.path().join("previous")).unwrap(),
            "old"
        );
    }

    #[test]
    fn metadata_failure_or_filename_collision_preserves_both_user_files() {
        let root = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir_in(root.path()).unwrap();
        let old = root.path().join("old.jar");
        let next = root.path().join("next.jar");
        fs::write(&old, "old").unwrap();
        fs::write(stage.path().join("download"), "new").unwrap();
        let error =
            replace_file(&old, &next, &stage, || Err("metadata write failed".into())).unwrap_err();
        assert_eq!(error, "metadata write failed");
        assert_eq!(fs::read_to_string(&old).unwrap(), "old");
        assert!(!next.exists());

        fs::write(&next, "another mod").unwrap();
        assert!(replace_file(&old, &next, &stage, || Ok(())).is_err());
        assert_eq!(fs::read_to_string(old).unwrap(), "old");
        assert_eq!(fs::read_to_string(next).unwrap(), "another mod");
    }
}
