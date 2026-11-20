//! 应用自更新：根据「设置 → 更新源」在运行时选择对象存储（存储桶）或
//! GitHub Releases 官方源，并用 `tauri-plugin-updater` 完成签名校验、下载与安装。

use crate::state::AppState;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 前端更新进度事件名
pub const PROGRESS_EVENT: &str = "app-update-progress";

/// 支持的更新源 id
pub const SOURCE_BUCKET: &str = "bucket";
pub const SOURCE_GITHUB: &str = "github";

fn release_version(value: &str) -> Option<semver::Version> {
    let value = value.trim();
    semver::Version::parse(value.strip_prefix(['v', 'V']).unwrap_or(value)).ok()
}

fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    Some(release_version(a)?.cmp_precedence(&release_version(b)?))
}

/// 存储桶更新源根地址（CI 会把构建产物和 `latest.json` 上传到该目录）。
///
/// 构建时通过 `QOOKIX_BUCKET_UPDATE_URL` 环境变量注入（见 `.github/workflows/build.yml`）。
/// 本地开发或手动部署时若未设置，请把下面的默认值改成你自己的对象存储目录，
/// 并保持与 CI 上传路径一致，否则 `<root>/latest.json` 将无法命中。
fn bucket_base() -> String {
    option_env!("QOOKIX_BUCKET_UPDATE_URL")
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| {
            // TODO: 部署时改成你的对象存储更新目录根地址（不含结尾斜杠）
            "https://your-bucket.example.com/qookix".to_string()
        })
}

/// GitHub Releases 官方源根地址（`latest.json` 随 release 附件发布在根目录）。
fn github_base() -> String {
    let repo = option_env!("QOOKIX_GITHUB_REPOSITORY")
        .or(option_env!("GITHUB_REPOSITORY"))
        .unwrap_or("weimosheng/QookiX-Launcher");
    format!("https://github.com/{repo}/releases/latest/download")
}

/// 根据源 id 解析出对应的 `latest.json` 更新清单地址。
fn manifest_url(source: &str) -> Result<url::Url, String> {
    let base = match source {
        SOURCE_GITHUB => github_base(),
        _ => bucket_base(),
    };
    url::Url::parse(&format!("{base}/latest.json"))
        .map_err(|e| format!("无效的更新源地址: {e}"))
}

/// 归一化更新源 id：仅 `github` 视为官方源，其余一律回退到存储桶。
fn normalize_source(source: &str) -> String {
    if source == SOURCE_GITHUB {
        SOURCE_GITHUB.to_string()
    } else {
        SOURCE_BUCKET.to_string()
    }
}

/// 决定实际使用的更新源：命令入参优先，否则取设置里的 `update_source`。
fn current_source(state: &AppState, override_source: Option<String>) -> String {
    match override_source {
        Some(s) => normalize_source(&s),
        None => normalize_source(&state.settings().update_source),
    }
}

fn build_updater(
    app: &AppHandle,
    source: &str,
) -> Result<tauri_plugin_updater::Updater, String> {
    app.updater_builder()
        .endpoints(vec![manifest_url(source)?])
        .map_err(|e| format!("无效的更新源地址: {e}"))?
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))
}

#[derive(Default)]
pub(crate) struct UpdateCache {
    decisions: VecDeque<CheckedUpdate>,
    ready: Option<DownloadedUpdate>,
}

struct CheckedUpdate {
    id: u64,
    source: String,
    update: Update,
}

impl UpdateCache {
    fn remember(&mut self, source: &str, update: Update, new_id: impl FnOnce() -> u64) -> u64 {
        let id = match self.decisions.iter().position(|checked| {
            checked.source == source
                && checked.update.version == update.version
                && checked.update.download_url == update.download_url
                && checked.update.signature == update.signature
                && checked.update.raw_json == update.raw_json
        }) {
            Some(index) => self.decisions.remove(index).map_or_else(new_id, |checked| checked.id),
            None => new_id(),
        };
        self.decisions.push_back(CheckedUpdate { id, source: source.to_string(), update });
        if self.decisions.len() > 16 {
            self.decisions.pop_front();
        }
        id
    }
}

struct DownloadedUpdate {
    decision_id: u64,
    update: Update,
    bytes: Vec<u8>,
}

fn update_lock_key(state: &AppState) -> PathBuf {
    state.root.join(".app-update")
}

fn minimum_version(manifest: &serde_json::Value, offered: &str) -> Option<String> {
    let min = manifest.get("minVersion")?.as_str()?;
    let cmp = compare_versions(min, offered)?;
    (cmp != std::cmp::Ordering::Greater).then(|| min.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub available: bool,
    pub decision_id: Option<u64>,
    pub version: Option<String>,
    pub current_version: Option<String>,
    pub body: Option<String>,
    pub download_url: Option<String>,
    pub source: String,
    /// 当前版本低于 `latest.json` 的 `minVersion` 时为 `true`：必须更新、不可忽略。
    pub force_update: bool,
    /// 触发强制更新的最低版本（前端用于展示提示文案）。
    pub force_min_version: Option<String>,
}

/// 检查所选更新源是否有新版本。`source` 缺省时使用设置中的 `update_source`。
#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    source: Option<String>,
) -> Result<UpdateInfo, String> {
    let source = current_source(&state, source);
    let updater = build_updater(&app, &source)?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;
    Ok(match update {
        Some(u) => {
            let min_version = minimum_version(&u.raw_json, &u.version);
            let force = min_version.as_ref().is_some_and(|min| {
                compare_versions(&u.current_version, min) == Some(std::cmp::Ordering::Less)
            });
            let mut info = UpdateInfo {
                available: true,
                decision_id: None,
                version: Some(u.version.clone()),
                current_version: Some(u.current_version.clone()),
                body: u.body.clone(),
                download_url: Some(u.download_url.to_string()),
                source,
                force_update: force,
                force_min_version: min_version,
            };
            let mut cache = state.pending_update.lock().unwrap_or_else(|e| e.into_inner());
            info.decision_id = Some(cache.remember(&info.source, u, || state.next_task_id()));
            info
        }
        None => UpdateInfo {
            available: false,
            decision_id: None,
            version: None,
            current_version: None,
            body: None,
            download_url: None,
            source,
            force_update: false,
            force_min_version: None,
        },
    })
}

/// 下载已经检查并确认的更新。安装由用户确认重启后执行。
#[tauri::command]
pub async fn download_update(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    decision_id: u64,
) -> Result<UpdateReceipt, String> {
    let activity = state.jobs.download_lock(update_lock_key(&state));
    let _guard = activity.try_lock().map_err(|_| "另一项启动器更新正在进行")?;
    let update = {
        let cache = state.pending_update.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ready) = cache.ready.as_ref().filter(|ready| ready.decision_id == decision_id) {
            return Ok(UpdateReceipt { decision_id, version: ready.update.version.clone() });
        }
        cache.decisions.iter().find(|checked| checked.id == decision_id)
            .map(|checked| checked.update.clone())
            .ok_or("该更新检查已过期，请重新检查更新")?
    };

    let handle = app.clone();
    let mut progress = DownloadProgress { decision_id, downloaded: 0, total: None };
    let bytes = update
        .download(
            |chunk, total| {
                progress.record_chunk(chunk, total);
                let _ = handle.emit(PROGRESS_EVENT, progress);
            },
            || {},
        )
        .await
        .map_err(|e| format!("下载更新失败: {e}"))?;

    progress.finish(bytes.len());
    let _ = app.emit(PROGRESS_EVENT, progress);
    let version = update.version.clone();
    state.pending_update.lock().unwrap_or_else(|e| e.into_inner()).ready =
        Some(DownloadedUpdate { decision_id, update, bytes });
    Ok(UpdateReceipt { decision_id, version })
}

/// 应用已下载的更新（校验签名后安装，Windows 上会静默安装并重启）。
#[tauri::command]
pub fn apply_app_update(state: tauri::State<'_, AppState>, decision_id: u64) -> Result<(), String> {
    let activity = state.jobs.download_lock(update_lock_key(&state));
    let _guard = activity.try_lock().map_err(|_| "另一项启动器更新正在进行")?;
    let ready = {
        let mut cache = state.pending_update.lock().unwrap_or_else(|e| e.into_inner());
        if cache.ready.as_ref().is_none_or(|ready| ready.decision_id != decision_id) {
            return Err("没有和本次确认匹配的已下载更新".into());
        }
        cache.ready.take().ok_or("没有已下载的更新")?
    };
    match ready.update.install(&ready.bytes) {
        Ok(()) => Ok(()),
        Err(err) => {
            state.pending_update.lock().unwrap_or_else(|e| e.into_inner()).ready = Some(ready);
            Err(format!("安装更新失败: {err}"))
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReceipt {
    pub decision_id: u64,
    pub version: String,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub decision_id: u64,
    pub downloaded: u64,
    pub total: Option<u64>,
}

impl DownloadProgress {
    fn record_chunk(&mut self, chunk: usize, total: Option<u64>) {
        self.downloaded = self.downloaded.saturating_add(chunk as u64);
        self.total = total.or(self.total);
    }

    fn finish(&mut self, verified_size: usize) {
        self.downloaded = verified_size as u64;
        self.total = Some(self.downloaded);
    }
}

/// 根据更新源拼接 changelog.json 地址。
fn changelog_url(source: &str) -> Result<String, String> {
    match source {
        SOURCE_GITHUB => Ok(format!("{}/changelog.json", github_base())),
        _ => Ok(format!("{}/changelog.json", bucket_base())),
    }
}

/// 获取 CHANGELOG.md 内容：优先国内镜像，失败回退 GitHub 官方源。
#[tauri::command]
pub async fn fetch_changelog(
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    for source in [SOURCE_BUCKET, SOURCE_GITHUB] {
        let url = match changelog_url(source) {
            Ok(u) => u,
            Err(_) => continue,
        };
        match crate::download::get_text(&state.client, &url).await {
            Ok(text) => return Ok(text),
            Err(_) => continue,
        }
    }
    Err("获取更新日志失败".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_matches_shared_semver_examples() {
        let examples: Vec<(String, String, Option<i8>)> =
            serde_json::from_str(include_str!("../../scripts/shared/release-version.test.json")).unwrap();
        for (left, right, expected) in examples {
            let order = compare_versions(&left, &right).map(|order| match order {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            });
            assert_eq!(order, expected, "{left} vs {right}");
        }
    }

    #[test]
    fn update_progress_accumulates_equal_and_unequal_chunks_and_finishes_verified() {
        let mut progress = DownloadProgress { decision_id: 1, downloaded: 0, total: None };
        progress.record_chunk(8, None);
        progress.record_chunk(8, None);
        assert_eq!(progress.downloaded, 16);
        assert_eq!(progress.total, None);
        progress.record_chunk(3, Some(19));
        assert_eq!(progress.downloaded, 19);
        assert_eq!(progress.total, Some(19));
        progress.finish(19);
        assert_eq!(progress.total, Some(19));
    }

    #[test]
    fn minimum_version_must_be_satisfied_by_the_offer_it_came_with() {
        assert_eq!(minimum_version(&serde_json::json!({"minVersion":"1.2.0"}), "1.2.0").as_deref(), Some("1.2.0"));
        assert_eq!(minimum_version(&serde_json::json!({"minVersion":"1.3.0"}), "1.2.0"), None);
        assert_eq!(minimum_version(&serde_json::json!({"minVersion":"unknown"}), "1.2.0"), None);
    }
}
