use crate::state::AppState;
use crate::task_events::{ActiveFile as ActiveFileProgress, DownloadProgress};
use crate::util::{file_sha1, file_sha512};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::Emitter;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct DownloadItem {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub size: Option<u64>,
    pub label: String,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DownloadStats {
    pub done: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Per-file progress slot shared between the download task and the periodic emitter.
#[derive(Clone)]
struct ActiveSlot {
    bytes_done: Arc<AtomicU64>,
    bytes_total: Arc<AtomicU64>,
    active: Arc<AtomicBool>,
}

fn progress_snapshot(
    slots: &[(String, ActiveSlot)],
    verified: u64,
) -> (u64, u64, Vec<ActiveFileProgress<'_>>) {
    let mut bytes_done = verified;
    let mut bytes_total = 0u64;
    let mut active = Vec::new();
    for (name, slot) in slots {
        let total = slot.bytes_total.load(Ordering::Relaxed);
        bytes_total = bytes_total.saturating_add(total);
        if slot.active.load(Ordering::Relaxed) {
            let done = slot.bytes_done.load(Ordering::Relaxed);
            bytes_done = bytes_done.saturating_add(done);
            active.push(ActiveFileProgress { name, bytes_done: done, bytes_total: total });
        }
    }
    (bytes_done, bytes_total.max(bytes_done), active)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Parallel downloader with a concurrency limit, sha1 verification and retries.
/// Streams every file and emits `download://progress` roughly every 800ms so the
/// frontend can show live average speed and remaining-file counts.
pub async fn download_many(
    app: tauri::AppHandle,
    state: &AppState,
    task_id: u64,
    phase: &str,
    items: Vec<DownloadItem>,
) -> Result<DownloadStats, String> {
    let total = items.len();
    let done = Arc::new(AtomicU64::new(0));
    let bytes_completed = Arc::new(AtomicU64::new(0));
    let cancel_flag = state.jobs.install_token(task_id);

    let (file_threads, chunk_threads, mirror_base) = {
        let s = state.settings();
        (
            s.download_threads.max(1),
            s.download_chunk_threads.max(1),
            crate::mirror::resolve_from(&s.mirror, &s.mirror_custom),
        )
    };
    let sem = Arc::new(Semaphore::new(file_threads));
    let client = state.client.clone();

    // Per-file progress slots for all items (read by the periodic emitter).
    let active_files: Vec<(String, ActiveSlot)> = items
        .iter()
        .map(|item| {
            (
                item.label.clone(),
                ActiveSlot {
                    bytes_done: Arc::new(AtomicU64::new(0)),
                    bytes_total: Arc::new(AtomicU64::new(item.size.unwrap_or(0))),
                    active: Arc::new(AtomicBool::new(false)),
                },
            )
        })
        .collect();
    let active_files = Arc::new(active_files);

    // periodic emitter (drives smooth speed display)
    let phase_owned = phase.to_string();
    let phase_e = phase_owned.clone();
    let app2 = app.clone();
    let done_e = done.clone();
    let bc_e = bytes_completed.clone();
    let cancel_e = cancel_flag.clone();
    let af_e = active_files.clone();
    let emitter = tauri::async_runtime::spawn(async move {
        loop {
            if cancel_e.is_cancelled() {
                break;
            }
            let d = done_e.load(Ordering::Relaxed);
            if d >= total as u64 {
                break;
            }
            let verified = bc_e.load(Ordering::Relaxed);
            let (bytes_done, bytes_total, active) = progress_snapshot(&af_e, verified);
            let _ = app2.emit(
                "download://progress",
                DownloadProgress {
                    task_id,
                    phase: &phase_e,
                    done: d,
                    total,
                    current: "",
                    ok: true,
                    bytes_done,
                    bytes_total,
                    verified_bytes: verified,
                    active_files: Some(active),
                    ts: now_ms(),
                },
            );
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    });

    let mut handles = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        let permit = tokio::select! {
            _ = cancel_flag.cancelled() => break,
            permit = sem.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => break,
            },
        };
        let client = client.clone();
        let app = app.clone();
        let done = done.clone();
        let bytes_completed = bytes_completed.clone();
        let cancel = cancel_flag.clone();
        let phase = phase_owned.clone();
        let mirror_base = mirror_base.clone();
        let my_done = active_files[i].1.bytes_done.clone();
        let my_total = active_files[i].1.bytes_total.clone();
        let my_active = active_files[i].1.active.clone();
        let slots = active_files.clone();
        let staging = state.jobs.download_lock(with_suffix(&item.dest, ".part"));
        handles.push(tauri::async_runtime::spawn(async move {
            let _permit = permit;
            let _staging = tokio::select! {
                _ = cancel.cancelled() => return Err("下载已取消".into()),
                guard = staging.lock() => guard,
            };
            my_active.store(true, Ordering::Relaxed);
            let mut result = Err(format!("not attempted: {}", item.label));
            let on_progress = |written: u64, length: u64| {
                my_done.store(written, Ordering::Relaxed);
                if length > 0 {
                    my_total.store(length, Ordering::Relaxed);
                }
            };
            for attempt in 0..3 {
                if cancel.is_cancelled() {
                    result = Err("已取消".into());
                    break;
                }
                my_done.store(0, Ordering::Relaxed);
                match download_one(&client, &item, &mirror_base, &on_progress, chunk_threads, &cancel).await {
                    Ok(actual_size) => {
                        result = Ok(actual_size);
                        break;
                    }
                    Err(e) => {
                        let permanent = e.contains("HTTP 4") || e.contains("HTTP 5");
                        result = Err(e);
                        if permanent || cancel.is_cancelled() {
                            break;
                        }
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = tokio::time::sleep(std::time::Duration::from_millis(400 * (attempt + 1) as u64)) => {}
                        }
                    }
                }
            }
            my_active.store(false, Ordering::Relaxed);
            if let Ok(size) = result {
                my_total.store(size, Ordering::Relaxed);
                bytes_completed.fetch_add(size, Ordering::Relaxed);
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            let verified = bytes_completed.load(Ordering::Relaxed);
            let (bytes_done, bytes_total, active) = progress_snapshot(&slots, verified);
            let _ = app.emit(
                "download://progress",
                DownloadProgress {
                    task_id,
                    phase: &phase,
                    done: d,
                    total,
                    current: &item.label,
                    ok: result.is_ok(),
                    bytes_done,
                    bytes_total,
                    verified_bytes: verified,
                    active_files: Some(active),
                    ts: now_ms(),
                },
            );
            result.map(|_| ()).map_err(|error| format!("{}: {error} ({})", item.label, item.url))
        }));
    }

    let mut errors: Vec<String> = Vec::new();
    for h in handles {
        match h.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e),
            Err(e) => errors.push(e.to_string()),
        }
    }
    emitter.abort();
    let _ = emitter.await;
    if cancel_flag.is_cancelled() {
        return Err("下载已取消".into());
    }
    if !errors.is_empty() {
        let sample: Vec<String> = errors.iter().take(5).cloned().collect();
        return Err(format!("{} 个文件下载失败: {}", errors.len(), sample.join("; ")));
    }
    let verified = bytes_completed.load(Ordering::Relaxed);
    let (_, final_bt, _) = progress_snapshot(&active_files, verified);
    Ok(DownloadStats {
        done: done.load(Ordering::Relaxed) as usize,
        total,
        bytes_done: verified,
        bytes_total: final_bt,
    })
}

/// Files at least this large are eligible for parallel chunked download (8 MB).
const CHUNK_THRESHOLD: u64 = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct ResumeState {
    source: String,
    etag: String,
    chunks: Option<ChunkState>,
}

#[derive(Deserialize, Serialize)]
struct ChunkState {
    size: u64,
    done: Vec<bool>,
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn strong_etag(headers: &reqwest::header::HeaderMap) -> Option<&str> {
    let tag = headers.get(reqwest::header::ETAG)?.to_str().ok()?;
    (tag.len() >= 2 && tag.starts_with('"') && tag.ends_with('"')).then_some(tag)
}

pub(crate) fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, whole) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, whole) = (start.parse().ok()?, end.parse().ok()?, whole.parse().ok()?);
    (start <= end && end < whole).then_some((start, end, whole))
}

fn content_range(response: &reqwest::Response) -> Option<(u64, u64, u64)> {
    let range = response.headers().get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    parse_content_range(range)
}

async fn probe_range_identity(client: &reqwest::Client, url: &str, expected_size: u64) -> Option<String> {
    let response = client.head(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .timeout(std::time::Duration::from_secs(10))
        .send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let length = response.headers().get(reqwest::header::CONTENT_LENGTH)?.to_str().ok()?.parse::<u64>().ok()?;
    let ranges = response.headers().get(reqwest::header::ACCEPT_RANGES)?.to_str().ok()?;
    if length != expected_size || !ranges.eq_ignore_ascii_case("bytes") {
        return None;
    }
    strong_etag(response.headers()).map(str::to_owned)
}

fn partial_length(part: &Path) -> Result<u64, String> {
    match std::fs::symlink_metadata(part) {
        Ok(metadata) if metadata.is_file() && !crate::fsutil::is_link_or_reparse_point(&metadata) => Ok(metadata.len()),
        Ok(_) => Err("下载暂存目标不是普通文件".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error.to_string()),
    }
}

fn read_resume(part: &Path) -> Option<ResumeState> {
    let data = std::fs::read(with_suffix(part, ".resume")).ok()?;
    serde_json::from_slice(&data).ok()
}

fn save_resume(part: &Path, state: &ResumeState) -> Result<(), String> {
    let data = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    crate::fsutil::atomic_write(&with_suffix(part, ".resume"), &data)
        .map_err(|error| format!("保存下载续传状态失败: {error}"))
}

fn remove_resume(part: &Path) {
    let path = with_suffix(part, ".resume");
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            eprintln!("清理下载续传状态 {} 失败: {error}", path.display());
        }
        _ => {}
    }
}

fn clear_partial(part: &Path) -> Result<(), String> {
    remove_resume(part);
    match std::fs::remove_file(part) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("清理旧的下载暂存文件失败: {error}")),
    }
}

/// Download a single file (streamed, with `.part` staging), verify sha1 when given.
/// Uses parallel chunked download when the server supports ranges and the file is large enough.
async fn download_one(
    client: &reqwest::Client,
    item: &DownloadItem,
    mirror_base: &str,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    chunk_threads: usize,
    cancel: &CancellationToken,
) -> Result<u64, String> {
    if let Some(parent) = item.dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir: {e}"))?;
    }
    if let Ok(metadata) = std::fs::metadata(&item.dest) {
        if metadata.is_file() {
            let valid = if let Some(hash) = &item.sha512 {
                file_sha512(&item.dest).is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
            } else if let Some(hash) = &item.sha1 {
                file_sha1(&item.dest).is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
            } else {
                item.size.is_none_or(|size| metadata.len() == size)
            };
            if valid {
                return Ok(metadata.len());
            }
        }
    }

    let part = with_suffix(&item.dest, ".part");
    // 支持断点续传：保留已有的 .part，由 fetch_and_verify 检测大小并从断点继续。
    // 仅当换源（镜像→官方）时才清空，避免不同来源的数据拼接错位。

    // 优先走镜像地址；镜像拉取/校验失败时回退到原始官方地址，
    // 这样即使镜像站缺失某个文件，安装也不会整体失败。
    let target = crate::mirror::map(mirror_base, &item.url);
    let mut result =
        fetch_and_verify(client, item, &target, &part, on_progress, chunk_threads, cancel).await;
    if result.is_err() && !cancel.is_cancelled() && target != item.url {
        clear_partial(&part)?;
        result =
            fetch_and_verify(client, item, &item.url, &part, on_progress, chunk_threads, cancel).await;
    }
    result?;
    let actual_size = tokio::fs::metadata(&part).await.map_err(|error| error.to_string())?.len();

    tokio::fs::rename(&part, &item.dest)
        .await
        .map_err(|e| format!("移动文件失败: {e}"))?;
    remove_resume(&part);
    Ok(actual_size)
}

/// 拉取单个文件（必要时分片并行）并校验哈希 / 大小。
async fn fetch_and_verify(
    client: &reqwest::Client,
    item: &DownloadItem,
    url: &str,
    part: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    chunk_threads: usize,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let chunk_resource = match item.size.filter(|size| chunk_threads > 1 && *size >= CHUNK_THRESHOLD) {
        Some(size) => tokio::select! {
            _ = cancel.cancelled() => return Err("下载已取消".into()),
            etag = probe_range_identity(client, url, size) => etag.map(|etag| (size, etag)),
        },
        None => None,
    };
    if let Some((size, etag)) = chunk_resource {
        match download_chunked(client, url, part, size, chunk_threads, etag, on_progress, cancel).await {
            Ok(()) => {}
            Err(error) if cancel.is_cancelled() => return Err(error),
            Err(_) => {
                clear_partial(part)?;
                download_streamed(client, url, part, item.size, on_progress, cancel).await?;
            }
        }
    } else {
        download_streamed(client, url, part, item.size, on_progress, cancel).await?;
    }

    if let Some(sha) = &item.sha512 {
        let actual = file_sha512(part).ok_or("校验失败: 无法读取")?;
        if !actual.eq_ignore_ascii_case(sha) {
            clear_partial(part)?;
            return Err(format!("sha512 不匹配 (期望 {sha}, 实际 {actual})"));
        }
    } else if let Some(sha) = &item.sha1 {
        let actual = file_sha1(part).ok_or("校验失败: 无法读取")?;
        if !actual.eq_ignore_ascii_case(sha) {
            clear_partial(part)?;
            return Err(format!("sha1 不匹配 (期望 {sha}, 实际 {actual})"));
        }
    }
    if let Some(size) = item.size {
        let len = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        if len != size {
            clear_partial(part)?;
            return Err(format!("大小不匹配 (期望 {size}, 实际 {len})"));
        }
    }
    Ok(())
}

/// Stream download (single connection, no chunking).
/// Resumes from an existing `.part` file when the server supports Range requests.
async fn download_streamed(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    expected_size: Option<u64>,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<(), String> {
    let existing = partial_length(part)?;
    let mut resume = read_resume(part).filter(|state| {
        state.source == url && state.chunks.is_none() && existing > 0
            && expected_size.is_none_or(|size| existing < size)
    });

    let (response, offset, total) = loop {
        let mut request = client.get(url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .timeout(std::time::Duration::from_secs(300));
        if let Some(state) = &resume {
            request = request.header(reqwest::header::RANGE, format!("bytes={existing}-"))
                .header(reqwest::header::IF_RANGE, &state.etag);
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err("下载已取消".into()),
            result = request.send() => result.map_err(|error| format!("请求失败: {error}"))?,
        };
        if response.status() == reqwest::StatusCode::OK {
            let bytes = response.content_length();
            if expected_size.zip(bytes).is_some_and(|(expected, actual)| expected != actual) {
                return Err("下载响应大小与预期不符".into());
            }
            break (response, 0, expected_size.or(bytes));
        }
        if let Some(state) = &resume {
            if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                if let Some((start, end, whole)) = content_range(&response) {
                    let remaining = whole - start;
                    if start == existing && end + 1 == whole
                        && expected_size.is_none_or(|size| size == whole)
                        && response.content_length().is_none_or(|len| len == remaining)
                        && strong_etag(response.headers()) == Some(state.etag.as_str())
                    {
                        break (response, existing, Some(whole));
                    }
                }
            }
            if matches!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT | reqwest::StatusCode::RANGE_NOT_SATISFIABLE) {
                clear_partial(part)?;
                resume = None;
                continue;
            }
        }
        return Err(format!("HTTP {}: 响应不符合下载请求", response.status()));
    };

    let mut file = if offset != 0 {
        tokio::fs::OpenOptions::new().append(true).open(part).await
    } else {
        let file = tokio::fs::File::create(part).await;
        if let Some(etag) = strong_etag(response.headers()) {
            save_resume(part, &ResumeState { source: url.into(), etag: etag.into(), chunks: None })?;
        } else {
            remove_resume(part);
        }
        file
    }.map_err(|error| format!("打开暂存文件失败: {error}"))?;

    let mut stream = response.bytes_stream();
    let mut written = offset;
    while let Some(chunk) = tokio::select! {
        _ = cancel.cancelled() => None,
        chunk = stream.next() => chunk,
    } {
        let chunk = chunk.map_err(|error| format!("读取失败: {error}"))?;
        written = written.checked_add(chunk.len() as u64).ok_or("下载字节数溢出")?;
        if total.is_some_and(|size| written > size) {
            return Err("下载响应超过预期字节数".into());
        }
        file.write_all(&chunk).await.map_err(|error| format!("写入失败: {error}"))?;
        on_progress(written, total.unwrap_or(0));
    }
    file.flush().await.map_err(|error| format!("写入失败: {error}"))?;
    if cancel.is_cancelled() {
        return Err("下载已取消".into());
    }
    if total.is_some_and(|size| written != size) {
        return Err("下载响应未达到预期字节数".into());
    }
    Ok(())
}

/// Parallel chunked download using HTTP Range requests.
/// Resumes by skipping chunks already recorded as complete in the sidecar.
async fn download_chunked(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    total_size: u64,
    chunk_count: usize,
    etag: String,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<(), String> {
    let chunk_size = total_size / chunk_count as u64;

    let existing_size = partial_length(part)?;
    let state = read_resume(part).filter(|state| {
        existing_size == total_size && state.source == url && state.etag == etag
            && state.chunks.as_ref().is_some_and(|chunks| chunks.size == total_size
                && chunks.done.len() == chunk_count)
    });
    let state = match state {
        Some(state) => state,
        None => {
            clear_partial(part)?;
            let file = std::fs::File::create(part).map_err(|error| format!("创建分片暂存文件失败: {error}"))?;
            file.set_len(total_size).map_err(|error| format!("分片预分配失败: {error}"))?;
            let state = ResumeState {
                source: url.into(),
                etag: etag.clone(),
                chunks: Some(ChunkState { size: total_size, done: vec![false; chunk_count] }),
            };
            save_resume(part, &state)?;
            state
        }
    };
    let done = Arc::new(std::sync::Mutex::new(state));

    // 已完成的字节数（用于进度显示）
    let written = Arc::new(AtomicU64::new(0));
    {
        let guard = done.lock().unwrap();
        for (i, &ok) in guard.chunks.as_ref().unwrap().done.iter().enumerate() {
            if ok {
                let start = i as u64 * chunk_size;
                let end = if i == chunk_count - 1 {
                    total_size - 1
                } else {
                    (i as u64 + 1) * chunk_size - 1
                };
                written.fetch_add(end - start + 1, Ordering::Relaxed);
            }
        }
    }

    let mut handles = tokio::task::JoinSet::new();

    for i in 0..chunk_count {
        // 已完成的片直接跳过
        {
            let guard = done.lock().unwrap();
            if guard.chunks.as_ref().unwrap().done[i] {
                continue;
            }
        }

        let start = i as u64 * chunk_size;
        let end = if i == chunk_count - 1 {
            total_size - 1
        } else {
            (i as u64 + 1) * chunk_size - 1
        };
        let len = end - start + 1;

        let client_c = client.clone();
        let url_c = url.to_string();
        let part_c = part.to_path_buf();
        let written_c = written.clone();
        let done_c = done.clone();

        let cancel = cancel.clone();
        let etag = etag.clone();
        handles.spawn(async move {
            let request = client_c
                .get(&url_c)
                .header("Accept-Encoding", "identity")
                .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                .header(reqwest::header::IF_RANGE, &etag)
                .timeout(std::time::Duration::from_secs(300))
                .send();
            let resp = tokio::select! {
                _ = cancel.cancelled() => return Err("下载已取消".into()),
                result = request => result.map_err(|e| format!("分片请求失败: {e}"))?,
            };
            let status = resp.status();
            if status.as_u16() != 206 {
                return Err(format!("分片 HTTP {status} (期望 206)"));
            }

            if content_range(&resp) != Some((start, end, total_size))
                || strong_etag(resp.headers()) != Some(etag.as_str())
                || resp.content_length().is_some_and(|length| length != len)
            {
                return Err(format!("分片 {i} 的资源身份或响应区间不匹配"));
            }

            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .open(&part_c)
                .await
                .map_err(|e| format!("打开文件失败: {e}"))?;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| format!("seek 失败: {e}"))?;

            let mut stream = resp.bytes_stream();
            let mut chunk_written = 0u64;
            while let Some(chunk) = tokio::select! {
                _ = cancel.cancelled() => None,
                chunk = stream.next() => chunk,
            } {
                let chunk = chunk.map_err(|e| format!("读取失败: {e}"))?;
                chunk_written = chunk_written.checked_add(chunk.len() as u64).ok_or("分片字节数溢出")?;
                if chunk_written > len {
                    return Err(format!("分片 {i} 超过已分配的区间"));
                }
                file.write_all(&chunk).await.map_err(|e| format!("写入失败: {e}"))?;
                written_c.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            }
            file.flush().await.map_err(|e| format!("写入失败: {e}"))?;
            if cancel.is_cancelled() {
                return Err("下载已取消".into());
            }
            if chunk_written != len {
                return Err(format!(
                    "分片 {i} 大小不匹配 (期望 {len}, 实际 {chunk_written})"
                ));
            }
            // 标记本分片完成并持久化 sidecar，支持断点续传
            let mut state = done_c.lock().unwrap();
            state.chunks.as_mut().unwrap().done[i] = true;
            save_resume(&part_c, &state)?;
            Ok(())
        });
    }

    // Wait for all chunks with periodic progress reporting
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
    let mut errors = Vec::new();
    loop {
        tokio::select! {
            result = handles.join_next() => match result {
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(e))) => errors.push(e),
                Some(Err(e)) => errors.push(e.to_string()),
                None => break,
            },
            _ = interval.tick() => {
                on_progress(written.load(Ordering::Relaxed), total_size);
            }
        }
    }
    // final progress flush
    on_progress(written.load(Ordering::Relaxed), total_size);

    if !errors.is_empty() {
        // 保留 .part 与 sidecar，供断点续传重试时跳过已完成分片
        return Err(format!("分片下载失败: {}", errors.join("; ")));
    }
    Ok(())
}

/// Simple single-file download used for metadata (no progress).
pub async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status} for {url}"));
    }
    resp.text().await.map_err(|e| e.to_string())
}

pub async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status} for {url}"));
    }
    resp.json::<T>().await.map_err(|e| format!("解析 {url} 失败: {e}"))
}

/// Ensure a file exists on disk (create if missing), returns path.
#[allow(dead_code)]
pub fn ensure_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    if !path.exists() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

#[cfg(test)]
mod diag {
    use super::*;

    /// 诊断：用与主程序一致的 client 配置下载 RLCraft 整合包（51MB），
    /// 复现「CurseForge 整合包点了安装没反应」。分别测 system 代理与 direct 直连。
    /// 运行：cargo test diag_cf_modpack_download -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn diag_cf_modpack_download() {
        let url = "https://edge.forgecdn.net/files/4612/979/RLCraft%201.12.2%20-%20Release%20v2.9.3.zip";
        for mode in ["system", "direct"] {
            let dest = std::env::temp_dir().join(format!("qookix-diag-rlcraft-{mode}.zip"));
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_file(dest.with_extension("part"));
            let item = DownloadItem {
                url: url.into(),
                dest: dest.clone(),
                sha1: None,
                sha512: None,
                size: Some(51324367),
                label: "RLCraft.zip".into(),
            };
            let client = crate::settings::http_client(mode, None);
            let started = std::time::Instant::now();
            let on_progress = |_w: u64, _c: u64| {};
            let r = download_one(&client, &item, "", &on_progress, 4, &CancellationToken::new()).await;
            let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            println!(
                "== [{mode}] 耗时 {:?}，结果 {:?}，文件 {}B ==",
                started.elapsed(),
                &r,
                size
            );
            let _ = std::fs::remove_file(&dest);
        }
    }

    /// 诊断： CurseForge API 请求（与 install_modpack 同一条 client/key 链路）。
    #[tokio::test]
    #[ignore]
    async fn diag_cf_api_get() {
        use std::sync::{Arc, Mutex, RwLock};
        let key = std::env::var("CURSEFORGE_API_KEY").unwrap_or_default();
        if key.is_empty() {
            println!("!! 未设置 CURSEFORGE_API_KEY，跳过");
            return;
        }
        for mode in ["system", "direct"] {
            let mut settings = crate::models::Settings::default();
            settings.curseforge_api_key = Some(key.clone());
            let root = std::env::temp_dir().join(format!("qookix-diag-cf-api-{mode}"));
            let _ = std::fs::create_dir_all(&root);
            let state = crate::state::AppState {
                root,
                settings: RwLock::new(settings),
                db: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
                client: crate::settings::http_client(mode, None),
                semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
                jobs: crate::runtime_jobs::RuntimeJobs::default(),


                ms_flow: Arc::new(Mutex::new(None)),
                shimu: Mutex::new(crate::shimu::ShimuAuth::default()),
                ygg_oauth: Mutex::new(crate::yggdrasil_oauth::YggOauthPending::default()),
                java_cache: Mutex::new(None),
                terracotta: Mutex::new(None),
                pending_update: Mutex::new(crate::updater::UpdateCache::default()),
                lan_sync: Mutex::new(None),
                qookix: Mutex::new(crate::qookix::QookixSession::default()),
                qookix_refresh: tokio::sync::Mutex::new(()),
            };
            let started = std::time::Instant::now();
            let r = crate::curseforge::search(&state, "RLCraft", "modpack", 0, 0, 3, "", "", "relevance").await;
            println!(
                "== [{mode}] CF API 耗时 {:?}，{} ==",
                started.elapsed(),
                match &r {
                    Ok(v) => format!("OK hits={}", v.get("hits").and_then(|h| h.as_array()).map(|a| a.len()).unwrap_or(0)),
                    Err(e) => format!("ERR: {e}"),
                }
            );
        }
    }
}


#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn ranges_are_validated_as_inclusive_intervals_inside_the_full_length() {
        assert_eq!(parse_content_range("bytes 4-7/8"), Some((4, 7, 8)));
        for value in ["bytes 4-8/8", "bytes 7-4/8", "bytes */8", "units 4-7/8", "bytes 0-1/*"] {
            assert_eq!(parse_content_range(value), None, "{value}");
        }
    }

    #[test]
    fn finished_failure_does_not_count_as_verified_and_totals_are_per_file() {
        let row = |length, done, active| ActiveSlot {
            bytes_done: Arc::new(AtomicU64::new(done)),
            bytes_total: Arc::new(AtomicU64::new(length)),
            active: Arc::new(AtomicBool::new(active)),
        };
        let slots = [
            ("failed".into(), row(18, 10, false)),
            ("verified".into(), row(9, 9, false)),
            ("unknown".into(), row(0, 5, true)),
        ];
        let (shown, total, active) = progress_snapshot(&slots, 9);
        assert_eq!((shown, total), (14, 27));
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "unknown");
    }

    #[test]
    fn staging_paths_keep_the_complete_destination_name() {
        assert_ne!(with_suffix(Path::new("same.jar"), ".part"), with_suffix(Path::new("same.json"), ".part"));
        assert_eq!(with_suffix(Path::new("same.jar"), ".part"), Path::new("same.jar.part"));
    }
}
