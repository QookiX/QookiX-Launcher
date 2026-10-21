//! 应用自更新：根据「设置 → 更新源」在运行时选择对象存储（存储桶）或
//! GitHub Releases 官方源，并用 `tauri-plugin-updater` 完成签名校验、下载与安装。

use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// 前端更新进度事件名
pub const PROGRESS_EVENT: &str = "app-update-progress";

/// 支持的更新源 id
pub const SOURCE_BUCKET: &str = "bucket";
pub const SOURCE_GITHUB: &str = "github";

/// 版本号比较：按 `.` / `-` / `+` 分段数值比较（与前端 `compareVersions` 对齐）。
/// 返回 -1 / 0 / 1。无法解析的段按 0 处理。
fn compare_versions(a: &str, b: &str) -> i8 {
    let parse = |s: &str| -> Vec<u64> {
        s.split(['.', '-', '+'])
            .map(|seg| seg.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let pa = parse(a);
    let pb = parse(b);
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let da = pa.get(i).copied().unwrap_or(0);
        let db = pb.get(i).copied().unwrap_or(0);
        if da != db {
            return if da < db { -1 } else { 1 };
        }
    }
    0
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

/// `latest.json` 里我们关心的自定义字段（插件不读这些，只读 version/platforms）。
/// `minVersion` 由 CI 通过 `QOOKIX_MIN_VERSION` 环境变量注入：低于此版本必须强制更新。
#[derive(serde::Deserialize)]
struct ManifestMeta {
    #[serde(rename = "minVersion")]
    min_version: Option<String>,
}

/// 拉取 `latest.json` 并提取 `minVersion`。网络失败时返回 `None`（不阻塞普通更新流程）。
async fn fetch_min_version(state: &AppState, source: &str) -> Option<String> {
    let url = manifest_url(source).ok()?;
    let text = crate::download::get_text(&state.client, url.as_str()).await.ok()?;
    let meta: ManifestMeta = serde_json::from_str(&text).ok()?;
    meta.min_version
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub available: bool,
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
            let current = u.current_version.clone();
            let min_version = fetch_min_version(&state, &source).await;
            // 强制更新：latest.json 声明了 minVersion 且当前版本严格低于它。
            let force = match &min_version {
                Some(m) => compare_versions(&current, m) < 0,
                None => false,
            };
            UpdateInfo {
                available: true,
                version: Some(u.version.clone()),
                current_version: Some(u.current_version.clone()),
                body: u.body.clone(),
                download_url: Some(u.download_url.to_string()),
                source,
                force_update: force,
                force_min_version: min_version,
            }
        }
        None => UpdateInfo {
            available: false,
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

/// 下载所选更新源的新版本，进度通过 `app-update-progress` 事件上报。
/// 下载完成后暂存，安装由 `apply_update` 在用户确认重启后执行。
#[tauri::command]
pub async fn download_update(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    source: Option<String>,
) -> Result<bool, String> {
    let source = current_source(&state, source);
    let updater = build_updater(&app, &source)?;
    let Some(update) = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?
    else {
        return Err("没有可用更新".into());
    };

    let handle = app.clone();
    let bytes = update
        .download(
            move |downloaded, total| {
                let _ = handle.emit(
                    PROGRESS_EVENT,
                    DownloadProgress {
                        downloaded: downloaded as u64,
                        total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| format!("下载更新失败: {e}"))?;

    *state.pending_update.lock().unwrap_or_else(|e| e.into_inner()) = Some((update, bytes));
    Ok(true)
}

/// 应用已下载的更新（校验签名后安装，Windows 上会静默安装并重启）。
#[tauri::command]
pub fn apply_app_update(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let Some((update, bytes)) = state.pending_update.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return Err("没有已下载的更新".into());
    };
    update
        .install(bytes)
        .map_err(|e| format!("安装更新失败: {e}"))
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
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
    fn compare_versions_basic() {
        assert_eq!(compare_versions("0.15.1", "0.15.2"), -1);
        assert_eq!(compare_versions("0.15.2", "0.15.1"), 1);
        assert_eq!(compare_versions("0.15.1", "0.15.1"), 0);
    }

    #[test]
    fn compare_versions_crosses_decimal_boundary() {
        // 0.10.0 > 0.9.0（字符串比较会搞错，数值比较不会）
        assert_eq!(compare_versions("0.9.0", "0.10.0"), -1);
        assert_eq!(compare_versions("0.10.0", "0.9.0"), 1);
    }

    #[test]
    fn compare_versions_handles_pre_release_suffixes() {
        // 与前端 compareVersions 对齐：按 ./-/+ 分段，非数字段当 0 处理。
        // "rc1" / "rc2" 都解析为 0，所以 0.15.1-rc1 == 0.15.1-rc2。
        assert_eq!(compare_versions("0.15.1-rc1", "0.15.1-rc2"), 0);
        // 但 0.15.1-rc1 < 0.15.2（第三段 1 < 2）
        assert_eq!(compare_versions("0.15.1-rc1", "0.15.2"), -1);
    }

    #[test]
    fn compare_versions_unparseable_segments_treated_as_zero() {
        assert_eq!(compare_versions("0.15.x", "0.15.0"), 0);
        assert_eq!(compare_versions("v0.15.1", "0.15.1"), 0);
    }
}
