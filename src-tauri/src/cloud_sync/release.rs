//! 云存档快照（GitHub Release）：创建并上传、列表、下载、删除与配额清理。
//!
//! - tag 形如 `world-<world_id>-<unix_secs>`
//! - Release body 存 JSON 元数据（世界名/版本/大小/sha256），列表无需下载内容
//! - 上传走 uploads.github.com；私有仓库下载走 assets API（带令牌）

use super::github;
use super::store;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// 解析 Release body 中的元数据 JSON（非本功能的返回 None）。
/// 云存档用 `world_id` 标归属，时光机早期只写 `instance_id`，两个都认。
fn parse_meta(release: &Value) -> Option<Value> {
    let body = release.get("body").and_then(|b| b.as_str())?;
    let meta: Value = serde_json::from_str(body.trim()).ok()?;
    if meta.get("world_id").is_some() || meta.get("instance_id").is_some() {
        Some(meta)
    } else {
        None
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SnapshotVolume {
    name: String,
    size: u64,
    sha256: Option<String>,
}

pub struct RestoreVolume {
    pub asset_id: u64,
    pub size: u64,
    pub sha256: Option<String>,
}

fn is_world_snapshot(meta: &Value) -> bool {
    meta.get("world_id").and_then(Value::as_str).is_some()
        && meta.get("type").and_then(Value::as_str) != Some("timemachine")
        && meta.get("shots").is_none()
}

fn world_volumes(meta: &Value, assets: &[(u64, u64, String)]) -> Result<Vec<RestoreVolume>, String> {
    if !is_world_snapshot(meta) {
        return Err("该快照不是世界存档".into());
    }
    let declared = meta.get("part_count").and_then(Value::as_u64).unwrap_or(1);
    if assets.is_empty() || declared != assets.len() as u64 {
        return Err("云快照分卷数量不完整".into());
    }
    let mut list: Vec<SnapshotVolume> = if let Some(parts) = meta.get("volumes") {
        let volumes: Vec<SnapshotVolume> = serde_json::from_value(parts.clone())
            .map_err(|error| format!("无效的云快照分卷清单: {error}"))?;
        if volumes.len() != assets.len() || volumes.iter().any(|volume| {
            !volume.sha256.as_ref().is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        }) {
            return Err("云快照分卷清单不完整或缺少校验值".into());
        }
        volumes
    } else {
        // Older snapshots stored only the first volume's hash. Count, exact
        // names, total size and ZIP integrity still guard every volume.
        (1..=declared).map(|index| {
            let name = if declared == 1 { "save.zip".to_string() } else { format!("save-part{index}.zip") };
            SnapshotVolume {
                name,
                size: 0,
                sha256: if index == 1 { meta.get("sha256").and_then(Value::as_str).map(str::to_owned) } else { None },
            }
        }).collect()
    };
    let ids = assets.iter().map(|asset| asset.0).collect::<std::collections::HashSet<_>>();
    let names = assets.iter().map(|asset| &asset.2).collect::<std::collections::HashSet<_>>();
    if ids.len() != assets.len() || names.len() != assets.len() {
        return Err("云快照包含重复的附件身份或文件名".into());
    }
    let mut remaining = assets.to_vec();
    let mut result = Vec::with_capacity(list.len());
    let mut total = 0u64;
    for volume in list.drain(..) {
        let at = remaining.iter().position(|(_, _, name)| *name == volume.name)
            .ok_or_else(|| format!("云快照缺少分卷 {}", volume.name))?;
        let (asset_id, size, _) = remaining.swap_remove(at);
        if asset_id == 0 || size == 0 || size > super::saves::MAX_ASSET_BYTES
            || (volume.size != 0 && volume.size != size)
        {
            return Err(format!("云快照分卷 {} 的身份或大小不符", volume.name));
        }
        total = total.checked_add(size).ok_or("云快照总大小超出限制")?;
        result.push(RestoreVolume { asset_id, size, sha256: volume.sha256 });
    }
    if !remaining.is_empty() || meta.get("size_bytes").and_then(Value::as_u64) != Some(total) {
        return Err("云快照附件总大小与清单不符".into());
    }
    Ok(result)
}

pub async fn restore_volumes(
    state: &AppState, token: &str, account: &str, repo_name: &str, release_id: u64,
) -> Result<Vec<RestoreVolume>, String> {
    let url = format!("{}/repos/{account}/{repo_name}/releases/{release_id}", github::API);
    let release = github::get(&state.client, &url, token).await?
        .ok_or("快照不存在或已被删除")?;
    if release.get("draft").and_then(Value::as_bool).unwrap_or(false) {
        return Err("快照尚未上传完成".into());
    }
    let meta = parse_meta(&release).ok_or("快照缺少存档元数据")?;
    let assets = list_assets(state, token, account, repo_name, release_id).await?;
    world_volumes(&meta, &assets)
}

/// 列出全部云存档快照（最新在前）。
pub async fn list(
    state: &AppState,
    token: &str,
    repo_name: &str,
    account: &str,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/repos/{account}/{repo_name}/releases", github::API);
    let Some(releases) = github::get_collection(&state.client, &url, token).await? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for rel in releases {
        let Some(meta) = parse_meta(&rel) else { continue };
        let release_id = rel.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
        let tag = rel.get("tag_name").and_then(|x| x.as_str()).unwrap_or("");
        let created = rel
            .get("created_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        // 附件信息（zip 本体，可能多个分卷；按名字排序保证 part 顺序）
        if rel.get("draft").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let mut assets = parse_assets(&rel);
        let declared = meta.get("part_count").and_then(Value::as_u64).unwrap_or(1);
        if declared > assets.len() as u64 {
            assets = list_assets(state, token, account, repo_name, release_id).await?;
        }
        if is_world_snapshot(&meta) && world_volumes(&meta, &assets).is_err() {
            continue;
        }
        let asset_ids: Vec<u64> = assets.iter().map(|a| a.0).collect();
        let asset_size: u64 = assets.iter().map(|a| a.1).sum();
        out.push(json!({
            "releaseId": release_id,
            "tag": tag,
            "createdAt": created,
            // 元数据里的 unix 秒（时光机按它排序/显示；GitHub 的 createdAt 是 ISO 串）
            "createdAtUnix": meta.get("created_at").cloned().unwrap_or(Value::Null),
            "assetId": asset_ids.first().copied().unwrap_or(0),
            "assetIds": asset_ids,
            "assetNames": assets.iter().map(|a| a.2.clone()).collect::<Vec<_>>(),
            "assetSize": asset_size,
            "partCount": meta.get("part_count").cloned().unwrap_or(json!(1)),
            // 时光机截图没有 world_id：归属键回落到 instance_id（按实例过滤要用）
            "worldId": meta
                .get("world_id")
                .cloned()
                .or_else(|| meta.get("instance_id").cloned())
                .unwrap_or(Value::Null),
            "worldName": meta.get("world_name").cloned().unwrap_or(Value::Null),
            "instanceId": meta.get("instance_id").cloned().unwrap_or(Value::Null),
            "instanceName": meta.get("instance_name").cloned().unwrap_or(Value::Null),
            "gameVersion": meta.get("game_version").cloned().unwrap_or(Value::Null),
            "sha256": meta.get("sha256").cloned().unwrap_or(Value::Null),
            // 时光机：这次截图是被什么触发的（interval / death / custom:<id> …）
            "trigger": meta.get("trigger").cloned().unwrap_or(Value::Null),
            // 时光机：一次攒批上传的多张截图 [{name, trigger, created_at}, …]
            "shots": meta.get("shots").cloned().unwrap_or(Value::Null),
        }));
    }
    Ok(out)
}

pub struct CreatedRelease {
    pub id: u64,
    pub assets: std::collections::HashMap<String, u64>,
}

/// 创建 Release 并上传压缩包（支持多分卷），返回已确认的 Release 和附件。
/// `parts`: (路径, 附件名)；`progress(sent, total)` 跨所有分卷连续回调。
pub async fn create_and_upload(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    world_id: &str,
    meta: &Value,
    parts: &[super::saves::PackedPart],
    // 附件 MIME：存档用 `application/zip`，时光机截图用 `image/png`
    content_type: &str,
    progress: impl Fn(u64, u64) + Send + Sync + 'static,
) -> Result<CreatedRelease, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if parts.is_empty() {
        return Err("没有可上传的快照附件".into());
    }
    let tag = format!("world-{world_id}-{stamp}-{}", uuid::Uuid::new_v4().simple());
    let mut meta_obj = meta.clone();
    meta_obj["part_count"] = json!(parts.len());
    if content_type == "application/zip" {
        let paths = parts.iter().map(|p| (p.path.clone(), p.asset_name.clone(), p.size)).collect::<Vec<_>>();
        let volumes = tokio::task::spawn_blocking(move || {
            paths.into_iter().map(|(path, name, size)| {
                super::saves::sha256_file(&path).map(|sha256| SnapshotVolume { name, size, sha256: Some(sha256) })
            }).collect::<Result<Vec<_>, String>>()
        }).await.map_err(|error| error.to_string())??;
        meta_obj["sha256"] = json!(volumes[0].sha256);
        meta_obj["volumes"] = json!(volumes);
    }
    let body = serde_json::to_string(&meta_obj).map_err(|error| error.to_string())?;
    let create_url = format!(
        "{}/repos/{}/{}/releases",
        github::API,
        account,
        repo_name
    );
    let rel = github::send_json(
        &state.client,
        reqwest::Method::POST,
        &create_url,
        token,
        Some(&json!({ "tag_name": tag, "name": tag, "body": body, "draft": true })),
    )
    .await
    .map_err(|e| format!("创建 Release 失败: {e}"))?;
    let release_id = rel.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
    if release_id == 0 {
        return Err("创建 Release 失败：响应缺少 id".into());
    }

    let upload: Result<_, String> = async {
        let grand_total: u64 = parts.iter().map(|p| p.size).sum();
        let mut grand_sent = 0u64;
        let mut assets = std::collections::HashMap::with_capacity(parts.len());
        let progress = std::sync::Arc::new(progress);
        for part in parts {
            let base = grand_sent;
            progress(base, grand_total);

            let upload_url = format!(
                "{}/repos/{}/{}/releases/{}/assets?name={}",
                github::UPLOADS,
                account,
                repo_name,
                release_id,
                crate::modrinth::urlencode(&part.asset_name)
            );

            // 流式上传：分块读取压缩包，边读边回调进度
            let total = part.size;
            let file = tokio::fs::File::open(&part.path)
                .await
                .map_err(|e| format!("打开压缩包失败: {e}"))?;
            let reader = tokio::io::BufReader::with_capacity(256 * 1024, file);
            use tokio::io::AsyncReadExt;
            let progress2 = progress.clone();
            let stream = futures_util::stream::unfold(
                (reader, 0u64, progress2),
                move |(mut r, sent, progress)| async move {
                    let mut buf = vec![0u8; 256 * 1024];
                    match r.read(&mut buf).await {
                        Ok(0) => None,
                        Ok(n) => {
                            let sent = sent + n as u64;
                            progress(base + sent, grand_total);
                            Some((
                                Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::copy_from_slice(&buf[..n])),
                                (r, sent, progress),
                            ))
                        }
                        Err(e) => Some((Err(e), (r, sent, progress))),
                    }
                },
            );

            let resp = state
                .client
                .post(&upload_url)
                .header("User-Agent", github::UA)
                .header("Accept", "application/vnd.github+json")
                .header(
                    "Content-Type",
                    // 同一批里混着图片和 shots.json：按扩展名给 MIME
                    if part.asset_name.ends_with(".json") {
                        "application/json"
                    } else {
                        content_type
                    },
                )
                .header("Content-Length", total)
                .bearer_auth(token)
                .body(reqwest::Body::wrap_stream(stream))
                .send()
                .await
                .map_err(|e| format!("上传压缩包失败: {e}"))?;
            if !resp.status().is_success() {
                let code = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                return Err(format!(
                    "上传压缩包失败（HTTP {code}）: {}",
                    text.chars().take(200).collect::<String>()
                ));
            }
            let uploaded: Value = resp.json().await.map_err(|error| format!("无法核对上传附件: {error}"))?;
            let asset_id = uploaded.get("id").and_then(Value::as_u64).unwrap_or(0);
            if uploaded.get("state").and_then(Value::as_str) != Some("uploaded")
                || uploaded.get("name").and_then(Value::as_str) != Some(part.asset_name.as_str())
                || uploaded.get("size").and_then(Value::as_u64) != Some(total)
                || asset_id == 0
            {
                return Err(format!("云端附件 {} 与上传分卷不匹配", part.asset_name));
            }
            assets.insert(part.asset_name.clone(), asset_id);
            grand_sent = base + total;
            progress(grand_sent, grand_total);
        }
        Ok(assets)
    }.await;
    let assets = match upload {
        Ok(assets) => assets,
        Err(error) => {
            return match delete(state, token, account, repo_name, release_id).await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}。草稿清理失败: {cleanup}")),
            };
        }
    };
    let publish_url = format!("{create_url}/{release_id}");
    let published = github::send_json(&state.client, reqwest::Method::PATCH, &publish_url, token, Some(&json!({ "draft": false })))
        .await
        .map_err(|error| format!("发布已上传的云快照失败: {error}"))?;
    if published.get("draft").and_then(Value::as_bool) != Some(false)
        || published.get("id").and_then(Value::as_u64) != Some(release_id) {
        return Err("发布云快照后的状态无法确认".into());
    }
    Ok(CreatedRelease { id: release_id, assets })
}

/// 删除一个 Release（连带其 Asset）。
pub async fn delete(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    release_id: u64,
) -> Result<(), String> {
    let url = format!(
        "{}/repos/{}/{}/releases/{}",
        github::API,
        account,
        repo_name,
        release_id
    );
    github::send_json(&state.client, reqwest::Method::DELETE, &url, token, None).await?;
    Ok(())
}

/// 删除单个附件（攒批上传的一个 Release 里只删其中一张截图时用）。
pub async fn delete_asset(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    asset_id: u64,
) -> Result<(), String> {
    let url = format!(
        "{}/repos/{}/{}/releases/assets/{}",
        github::API,
        account,
        repo_name,
        asset_id
    );
    github::send_json(&state.client, reqwest::Method::DELETE, &url, token, None).await?;
    Ok(())
}

/// 某个 Release 还剩哪些附件 id（删掉一张后判断要不要连 Release 一起删）。
pub async fn asset_ids(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    release_id: u64,
) -> Result<Vec<u64>, String> {
    Ok(list_assets(state, token, account, repo_name, release_id)
        .await?
        .into_iter()
        .map(|(id, _, _)| id)
        .collect())
}

/// 列出某个 Release 的全部附件（按名字排序，保证分卷顺序）。
pub async fn list_assets(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    release_id: u64,
) -> Result<Vec<(u64, u64, String)>, String> {
    let url = format!("{}/repos/{account}/{repo_name}/releases/{release_id}/assets", github::API);
    let rows = github::get_collection(&state.client, &url, token).await?
        .ok_or("快照不存在或已被删除")?;
    Ok(parse_asset_rows(&rows))
}

fn parse_assets(release: &Value) -> Vec<(u64, u64, String)> {
    release.get("assets").and_then(Value::as_array)
        .map_or_else(Vec::new, |rows| parse_asset_rows(rows))
}

fn parse_asset_rows(rows: &[Value]) -> Vec<(u64, u64, String)> {
    let mut assets: Vec<(u64, u64, String)> = rows.iter().filter_map(|asset| {
        Some((
            asset.get("id").and_then(Value::as_u64)?,
            asset.get("size").and_then(Value::as_u64)?,
            asset.get("name").and_then(Value::as_str)?.to_owned(),
        ))
    }).collect();
    assets.sort_by(|a, b| a.2.cmp(&b.2));
    assets
}

/// 下载 Asset 到本地文件（私有仓库需带令牌）。
/// `progress(received, total)` 会在下载过程中被回调。
pub async fn download_asset(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    asset_id: u64,
    dest: &Path,
    progress: impl Fn(u64, u64) + Send,
) -> Result<(), String> {
    download_asset_sized(state, token, account, repo_name, asset_id, dest, None, progress).await
}

pub async fn download_asset_sized(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    asset_id: u64,
    dest: &Path,
    expected_size: Option<u64>,
    progress: impl Fn(u64, u64) + Send,
) -> Result<(), String> {
    let url = format!(
        "{}/repos/{}/{}/releases/assets/{}",
        github::API,
        account,
        repo_name,
        asset_id
    );
    let resp = state
        .client
        .get(&url)
        .header("User-Agent", github::UA)
        .header("Accept", "application/octet-stream")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败：HTTP {}", resp.status().as_u16()));
    }
    let content_length = resp.content_length();
    if expected_size.zip(content_length).is_some_and(|(expected, actual)| expected != actual) {
        return Err("快照附件响应大小与清单不符".into());
    }
    let total = expected_size.or(content_length).unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| format!("创建文件失败: {e}"))?;
    let mut received = 0u64;
    let mut resp = resp;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("读取下载内容失败: {e}"))?
    {
        use tokio::io::AsyncWriteExt;
        received = received.checked_add(chunk.len() as u64).ok_or("云快照分卷大小超限")?;
        if expected_size.is_some_and(|expected| received > expected) {
            return Err("实收快照分卷比清单记录更大".into());
        }
        file.write_all(&chunk).await.map_err(|e| format!("写入文件失败: {e}"))?;
        progress(received, total);
    }
    if expected_size.is_some_and(|expected| received != expected) {
        return Err("快照分卷下载不完整".into());
    }
    use tokio::io::AsyncWriteExt;
    file.flush().await.map_err(|error| error.to_string())?;
    Ok(())
}

/// 配额清理：某世界的快照超过上限时，从最旧的开始删除。
/// `snapshots` 为 list() 的结果（最新在前）。
pub async fn cleanup_quota(
    state: &AppState,
    token: &str,
    account: &str,
    repo_name: &str,
    world_id: &str,
    snapshots: &[Value],
    keep: u32,
) -> Result<u32, String> {
    let mine: Vec<&Value> = snapshots
        .iter()
        .filter(|s| s.get("worldId").and_then(|v| v.as_str()) == Some(world_id))
        .collect();
    let keep = keep.max(1) as usize;
    if mine.len() <= keep {
        return Ok(0);
    }
    let mut deleted = 0u32;
    for s in mine.iter().skip(keep) {
        if let Some(id) = s.get("releaseId").and_then(|v| v.as_u64()) {
            if delete(state, token, account, repo_name, id).await.is_ok() {
                deleted += 1;
            }
        }
    }
    Ok(deleted)
}

/// 读取本地保存的每个世界快照上限（默认 5）。
pub fn keep_per_world(state: &AppState) -> u32 {
    let cs = store::load(state);
    if cs.keep_per_world == 0 {
        5
    } else {
        cs.keep_per_world
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_snapshot_checks_volume_names_sizes_ids_and_sha256() {
        let hash = "a".repeat(64);
        let metadata = json!({
            "world_id": "world",
            "part_count": 2,
            "size_bytes": 30,
            "volumes": [
                { "name": "save-part1.zip", "size": 10, "sha256": hash },
                { "name": "save-part2.zip", "size": 20, "sha256": "b".repeat(64) }
            ]
        });
        let first = (17, 10, "save-part1.zip".into());
        let second = (18, 20, "save-part2.zip".into());
        let selected = world_volumes(&metadata, &[second.clone(), first.clone()]).unwrap();
        assert_eq!(selected[0].asset_id, 17);
        assert_eq!(selected[1].asset_id, 18);
        assert_eq!(selected[0].sha256.as_deref(), Some(hash.as_str()));
        assert!(world_volumes(&metadata, &[first.clone()]).is_err());
        assert!(world_volumes(&metadata, &[first.clone(), (18, 20, "wrong.zip".into())]).is_err());
        assert!(world_volumes(&metadata, &[first.clone(), (18, 19, second.2.clone())]).is_err());
        assert!(world_volumes(&metadata, &[first, (17, 20, second.2)]).is_err());
    }

    #[test]
    fn legacy_world_snapshot_retains_first_volume_hash() {
        let hash = "a".repeat(64);
        let metadata = json!({ "world_id": "world", "part_count": 2, "size_bytes": 30, "sha256": hash });
        let selected = world_volumes(&metadata, &[(1, 10, "save-part1.zip".into()), (2, 20, "save-part2.zip".into())]).unwrap();
        assert_eq!(selected[0].sha256.as_deref(), Some(hash.as_str()));
        assert!(selected[1].sha256.is_none());
        assert!(!is_world_snapshot(&json!({ "type": "timemachine", "world_id": "example" })));
    }
}
