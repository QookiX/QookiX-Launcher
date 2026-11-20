//! 材质包（资源包）预览。
//!
//! 只读不写：打开 `.zip` / 文件夹，解析 `pack.mcmeta` 与 `pack.png`，
//! 枚举 `assets/<ns>/textures/**` 下的贴图清单；贴图本体由前端按需
//! 逐张拉取（懒加载），避免一次性把几百 MB 的解码结果塞进内存。
//!
//! 会话（session）内保留已打开的 zip，省去每张图都重新解析一次中央目录；
//! 页面卸载时前端调 `resourcepack_close` 释放。

use super::preview_sessions::PreviewSessions;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 单张贴图上限（防止误开超大文件把内存吃满）
const MAX_TEXTURE_BYTES: u64 = 48 * 1024 * 1024;
/// 文本条目（pack.mcmeta / .mcmeta）上限
const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
/// 清单里最多收录多少张贴图（极端材质的整包导出会到几万张）
const MAX_TEXTURES: usize = 40_000;
/// 同时保留的会话数
const MAX_SESSIONS: usize = 6;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePackTexture {
    /// 包内相对路径，前端读取时原样传回
    pub path: String,
    /// 展示名（去掉 `assets/<ns>/textures/` 前缀与 `.png` 后缀）
    pub label: String,
    /// 文件名（不含目录）
    pub file_name: String,
    /// 分类目录：block / item / gui / entity / ...
    pub category: String,
    /// 命名空间（assets 下的一级目录）
    pub namespace: String,
    pub width: u32,
    pub height: u32,
    /// 动画帧数，静态贴图为 1
    pub frames: u32,
    pub bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePackManifest {
    pub session_id: String,
    pub path: String,
    /// zip / folder
    pub kind: String,
    /// 展示名（zip 文件名或文件夹名）
    pub name: String,
    pub pack_format: Option<i64>,
    /// 支持格式区间，例如 "15 – 18"
    pub supported_formats: Option<String>,
    pub description: String,
    /// pack.png 的 data URL
    pub icon: Option<String>,
    pub file_size: u64,
    pub textures: Vec<ResourcePackTexture>,
    pub namespaces: Vec<String>,
    /// 出现过的贴图宽度（升序），用于 "16x / 32x" 这类分辨率标记
    pub resolutions: Vec<u32>,
    pub animated: usize,
    pub models: usize,
    pub sounds: usize,
    pub langs: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceResourcePack {
    pub path: String,
    pub name: String,
    pub size: u64,
}

// ------------------------------------------------------------------ 数据源 --

struct PackSource {
    /// Some → zip；None → 文件夹
    archive: Option<zip::ZipArchive<BufReader<File>>>,
    base: PathBuf,
    /// zip 内（或文件夹内）材质包根目录前缀，形如 "MyPack/" 或 ""
    prefix: String,
}

fn validate_rel(rel: &str) -> Result<(), String> {
    if rel.is_empty() || rel.len() > 4096 {
        return Err("非法条目".into());
    }
    if rel.starts_with('/') || rel.contains('\\') || rel.contains('\0') {
        return Err("非法条目".into());
    }
    if rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err("非法条目".into());
    }
    // Windows 盘符 / 备用数据流
    if rel.contains(':') {
        return Err("非法条目".into());
    }
    Ok(())
}

impl PackSource {
    fn read(&mut self, rel: &str, limit: u64) -> Result<Vec<u8>, String> {
        validate_rel(rel)?;
        match self.archive.as_mut() {
            Some(archive) => {
                let name = if self.prefix.is_empty() {
                    rel.to_string()
                } else {
                    format!("{}{}", self.prefix, rel)
                };
                let mut entry = archive
                    .by_name(&name)
                    .map_err(|_| format!("材质包内不存在：{rel}"))?;
                if entry.size() > limit {
                    return Err("文件过大，已跳过".into());
                }
                let mut buf = Vec::with_capacity(entry.size().min(limit) as usize);
                entry
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("解压失败: {e}"))?;
                Ok(buf)
            }
            None => {
                let path = self.base.join(&self.prefix).join(rel);
                let meta = std::fs::metadata(&path).map_err(|_| format!("文件不存在：{rel}"))?;
                if meta.len() > limit {
                    return Err("文件过大，已跳过".into());
                }
                std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))
            }
        }
    }

    /// 只读文件头（PNG 的 IHDR 够用了），不把整张图解压出来
    fn read_head(&mut self, rel: &str, len: usize) -> Option<Vec<u8>> {
        validate_rel(rel).ok()?;
        let mut buf = vec![0u8; len];
        match self.archive.as_mut() {
            Some(archive) => {
                let name = if self.prefix.is_empty() {
                    rel.to_string()
                } else {
                    format!("{}{}", self.prefix, rel)
                };
                let mut entry = archive.by_name(&name).ok()?;
                let mut read = 0usize;
                while read < buf.len() {
                    match entry.read(&mut buf[read..]) {
                        Ok(0) => break,
                        Ok(n) => read += n,
                        Err(_) => return None,
                    }
                }
                buf.truncate(read);
                Some(buf)
            }
            None => {
                let path = self.base.join(&self.prefix).join(rel);
                let mut file = File::open(path).ok()?;
                file.seek(SeekFrom::Start(0)).ok()?;
                let mut read = 0usize;
                while read < buf.len() {
                    match file.read(&mut buf[read..]) {
                        Ok(0) => break,
                        Ok(n) => read += n,
                        Err(_) => return None,
                    }
                }
                buf.truncate(read);
                Some(buf)
            }
        }
    }
}

/// 列出一个 zip / 文件夹里的全部文件（相对路径, 未压缩大小）
fn collect_files(path: &Path) -> Result<(Vec<(String, u64)>, Option<zip::ZipArchive<BufReader<File>>>), String> {
    if path.is_dir() {
        let mut out = Vec::new();
        walk_dir(path, "", 0, &mut out)?;
        return Ok((out, None));
    }
    let file = File::open(path).map_err(|e| format!("打开文件失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| format!("这不是有效的 zip 压缩包: {e}"))?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("读取压缩包条目失败: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        // 用反斜杠当分隔符的 zip 既不规范，游戏本身也读不出来
        if name.contains('\\') {
            continue;
        }
        out.push((name, entry.size()));
    }
    Ok((out, Some(archive)))
}

fn walk_dir(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, u64)>) -> Result<(), String> {
    if depth > 24 || out.len() > 60_000 {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        // 符号链接可能指回上层目录，形成环
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if file_type.is_dir() {
            walk_dir(&entry.path(), &rel, depth + 1, out)?;
        } else if file_type.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((rel, size));
        }
    }
    Ok(())
}

/// 所有带 pack.mcmeta 的目录（"" 表示根目录）
fn pack_roots(files: &[(String, u64)]) -> Vec<String> {
    let mut roots: Vec<String> = files
        .iter()
        .filter_map(|(name, _)| {
            name.strip_suffix("pack.mcmeta")
                .filter(|dir| dir.is_empty() || dir.ends_with('/'))
                .map(|dir| dir.to_string())
        })
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// 找到材质包根目录：优先根目录的 pack.mcmeta，其次最浅的一层。
/// 直接选了「resourcepacks」这种装着多个包的父目录时，明确报错而不是随便挑一个。
fn detect_prefix(files: &[(String, u64)]) -> Result<String, String> {
    let roots = pack_roots(files);
    if roots.is_empty() || roots.iter().any(|r| r.is_empty()) {
        return Ok(String::new());
    }
    let Some(depth) = roots.iter().map(|r| r.matches('/').count()).min() else {
        return Ok(String::new());
    };
    let shallow: Vec<&String> = roots
        .iter()
        .filter(|r| r.matches('/').count() == depth)
        .collect();
    if shallow.len() > 1 {
        return Err("该文件夹里包含多个材质包，请直接选择其中一个".into());
    }
    Ok(shallow[0].clone())
}

// ------------------------------------------------------------ 路径 / 文本 --

/// 拆出 (命名空间, textures/ 之后的相对路径)；不是贴图返回 None
fn split_texture(rel: &str) -> Option<(String, String)> {
    let lower = rel.to_ascii_lowercase();
    if !lower.ends_with(".png") {
        return None;
    }
    if let Some(rest) = rel.strip_prefix("assets/") {
        let mut it = rest.splitn(3, '/');
        let namespace = it.next()?;
        let middle = it.next()?;
        let tail = it.next()?;
        if !middle.eq_ignore_ascii_case("textures") || tail.is_empty() {
            return None;
        }
        return Some((namespace.to_string(), tail.to_string()));
    }
    // 1.6 之前的老材质包直接放在 textures/ 下
    if let Some(tail) = rel.strip_prefix("textures/") {
        if tail.is_empty() {
            return None;
        }
        return Some(("minecraft".to_string(), tail.to_string()));
    }
    None
}

fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || &head[0..8] != b"\x89PNG\r\n\x1a\n" || &head[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

/// 去掉 Minecraft 的 § 格式化代码
fn strip_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c == '\u{00a7}' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// `description` 可以是字符串，也可以是文本组件（对象 / 数组 / extra）
fn flatten_component(value: &Value, out: &mut String) {
    match value {
        Value::String(s) => out.push_str(s),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::Array(items) => {
            for item in items {
                flatten_component(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                out.push_str(text);
            } else if let Some(key) = map.get("translate").and_then(|v| v.as_str()) {
                // 没有语言文件可查，直接露出翻译键，比空白强
                out.push_str(key);
            } else if let Some(text) = map.get("fallback").and_then(|v| v.as_str()) {
                out.push_str(text);
            }
            if let Some(extra) = map.get("extra") {
                flatten_component(extra, out);
            }
            if let Some(with) = map.get("with") {
                flatten_component(with, out);
            }
        }
        _ => {}
    }
}

fn format_range(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(n) => Some(n.to_string()),
        Value::Array(items) if items.len() >= 2 => Some(format!(
            "{} – {}",
            items[0].as_i64().map(|v| v.to_string()).unwrap_or_else(|| items[0].to_string()),
            items[1].as_i64().map(|v| v.to_string()).unwrap_or_else(|| items[1].to_string())
        )),
        Value::Object(map) => {
            let min = map.get("min_inclusive").and_then(|v| v.as_i64());
            let max = map.get("max_inclusive").and_then(|v| v.as_i64());
            match (min, max) {
                (Some(a), Some(b)) => Some(format!("{a} – {b}")),
                (Some(a), None) => Some(a.to_string()),
                _ => None,
            }
        }
        _ => None,
    }
}

fn data_url(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes)
    ))
}

// ------------------------------------------------------------------ 会话 --

struct Session {
    source: Mutex<PackSource>,
}

static SESSIONS: PreviewSessions<Session> =
    PreviewSessions::new(MAX_SESSIONS, "会话已失效，请重新打开材质包");

// ------------------------------------------------------------------ 解析 --

fn open_pack(path_str: &str) -> Result<(ResourcePackManifest, PackSource), String> {
    let path = PathBuf::from(path_str);
    if !path.exists() {
        return Err("文件或文件夹不存在".into());
    }
    let kind = if path.is_dir() { "folder" } else { "zip" };
    let (files, archive) = collect_files(&path)?;
    if files.is_empty() {
        return Err("压缩包 / 文件夹是空的".into());
    }
    let prefix = detect_prefix(&files)?;
    let mut source = PackSource {
        archive,
        base: path.clone(),
        prefix: prefix.clone(),
    };

    // 去掉材质包根目录前缀
    let mut rel_files: Vec<(String, u64)> = files
        .iter()
        .filter_map(|(name, size)| {
            if prefix.is_empty() {
                Some((name.clone(), *size))
            } else {
                name.strip_prefix(&prefix).map(|rest| (rest.to_string(), *size))
            }
        })
        .filter(|(name, _)| !name.is_empty())
        .collect();
    rel_files.sort_by(|a, b| a.0.cmp(&b.0));

    let names: HashSet<String> = rel_files.iter().map(|(n, _)| n.clone()).collect();

    // —— pack.mcmeta ——
    let mut pack_format = None;
    let mut supported_formats = None;
    let mut description = String::new();
    if let Ok(bytes) = source.read("pack.mcmeta", MAX_TEXT_BYTES) {
        if let Ok(meta) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(pack) = meta.get("pack") {
                pack_format = pack.get("pack_format").and_then(|v| v.as_i64());
                supported_formats = format_range(pack.get("supported_formats")).or_else(|| {
                    // 1.21.9+ 的独立字段
                    let min = pack.get("min_format").and_then(|v| v.as_i64());
                    let max = pack.get("max_format").and_then(|v| v.as_i64());
                    match (min, max) {
                        (Some(a), Some(b)) => Some(format!("{a} – {b}")),
                        (Some(a), None) => Some(a.to_string()),
                        _ => None,
                    }
                });
                if let Some(desc) = pack.get("description") {
                    let mut raw = String::new();
                    flatten_component(desc, &mut raw);
                    description = strip_codes(&raw);
                }
            }
        }
    }

    let icon = source
        .read("pack.png", MAX_TEXTURE_BYTES)
        .ok()
        .and_then(|bytes| data_url(&bytes));

    // —— 贴图 / 其它资源统计 ——
    let mut textures = Vec::new();
    let mut resolutions = BTreeSet::new();
    let mut namespaces = BTreeSet::new();
    let mut animated = 0usize;
    let mut models = 0usize;
    let mut sounds = 0usize;
    let mut langs = 0usize;
    let mut total_size = 0u64;

    for (rel, size) in &rel_files {
        total_size += *size;
        let lower = rel.to_ascii_lowercase();
        if lower.starts_with("assets/") {
            if lower.contains("/models/") {
                models += 1;
            } else if lower.contains("/sounds/") {
                sounds += 1;
            } else if lower.contains("/lang/") && lower.ends_with(".json") {
                langs += 1;
            }
        }
        if textures.len() >= MAX_TEXTURES {
            continue;
        }
        let Some((namespace, tail)) = split_texture(rel) else {
            continue;
        };
        let Some(head) = source.read_head(rel, 33) else {
            continue;
        };
        let Some((width, height)) = png_size(&head) else {
            continue;
        };

        // 动画贴图：同名 .mcmeta 里带 animation；帧高默认取图宽
        let mut frames = 1u32;
        let mcmeta = format!("{rel}.mcmeta");
        if names.contains(&mcmeta) {
            if let Ok(bytes) = source.read(&mcmeta, MAX_TEXT_BYTES) {
                if let Ok(meta) = serde_json::from_slice::<Value>(&bytes) {
                    if let Some(anim) = meta.get("animation") {
                        if let Some(list) = anim.get("frames").and_then(|v| v.as_array()) {
                            if !list.is_empty() {
                                frames = list.len() as u32;
                            }
                        }
                        if frames <= 1 {
                            let frame_h = anim
                                .get("height")
                                .and_then(|v| v.as_u64())
                                .map(|v| v as u32)
                                .or_else(|| {
                                    anim.get("width").and_then(|v| v.as_u64()).map(|v| v as u32)
                                })
                                .unwrap_or(width);
                            if frame_h > 0 && height > frame_h {
                                frames = height / frame_h;
                            }
                        }
                    }
                }
            }
        }

        if frames > 1 {
            animated += 1;
        }
        resolutions.insert(width);
        namespaces.insert(namespace.clone());

        let label = tail[..tail.len() - 4].to_string();
        let file_name = label
            .rsplit('/')
            .next()
            .unwrap_or(&label)
            .to_string();
        let category = if label.contains('/') {
            label.split('/').next().unwrap_or("misc").to_ascii_lowercase()
        } else {
            "misc".to_string()
        };

        textures.push(ResourcePackTexture {
            path: rel.clone(),
            label,
            file_name,
            category,
            namespace,
            width,
            height,
            frames,
            bytes: *size,
        });
    }

    if textures.is_empty() {
        return Err("未在该文件内找到材质贴图，可能不是材质包".into());
    }

    let name = path
        .file_stem()
        .or_else(|| path.file_name())
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "resourcepack".to_string());

    let manifest = ResourcePackManifest {
        session_id: uuid::Uuid::new_v4().to_string(),
        path: path_str.to_string(),
        kind: kind.to_string(),
        name,
        pack_format,
        supported_formats,
        description,
        icon,
        file_size: total_size,
        textures,
        namespaces: namespaces.into_iter().collect(),
        resolutions: resolutions.into_iter().collect(),
        animated,
        models,
        sounds,
        langs,
    };
    Ok((manifest, source))
}

// ------------------------------------------------------------------ 命令 --

/// 打开材质包（.zip 或文件夹），返回清单并创建预览会话。
#[tauri::command]
pub async fn resourcepack_open(path: String) -> Result<ResourcePackManifest, String> {
    let (manifest, source) = tokio::task::spawn_blocking(move || open_pack(&path))
        .await
        .map_err(|e| e.to_string())??;
    SESSIONS.insert(
        manifest.session_id.clone(),
        Session {
            source: Mutex::new(source),
        },
    )?;
    Ok(manifest)
}

/// 读取单张贴图，返回 data URL。
#[tauri::command]
pub async fn resourcepack_texture(session_id: String, path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let session = SESSIONS.get(&session_id)?;
        let mut source = session
            .source
            .lock()
            .map_err(|_| "材质包会话不可用".to_string())?;
        let bytes = source.read(&path, MAX_TEXTURE_BYTES)?;
        data_url(&bytes).ok_or_else(|| "不是有效的 PNG 贴图".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 读取包内的文本条目（如 xxx.png.mcmeta），用于展示动画信息。
#[tauri::command]
pub async fn resourcepack_text(session_id: String, path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let session = SESSIONS.get(&session_id)?;
        let mut source = session
            .source
            .lock()
            .map_err(|_| "材质包会话不可用".to_string())?;
        let bytes = source.read(&path, MAX_TEXT_BYTES)?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 关闭会话，释放 zip 文件句柄。
#[tauri::command]
pub fn resourcepack_close(session_id: String) -> Result<(), String> {
    SESSIONS.close(&session_id)
}

/// 列出某个实例 `resourcepacks/` 目录下的材质包（zip / 文件夹）。
#[tauri::command]
pub fn resourcepack_list_instance_packs(
    state: tauri::State<'_, crate::state::AppState>,
    instance_id: String,
) -> Result<Vec<InstanceResourcePack>, String> {
    crate::instances::validate_instance_id(&instance_id)?;
    let dir = state.instances_dir().join(&instance_id).join("resourcepacks");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("读取目录失败: {e}"))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            // 未打包的材质包文件夹：只认带 pack.mcmeta 的
            if !path.join("pack.mcmeta").is_file() {
                continue;
            }
        } else {
            let ext = path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "zip" {
                continue;
            }
        }
        let name = path
            .file_stem()
            .or_else(|| path.file_name())
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_default();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push(InstanceResourcePack {
            path: path.to_string_lossy().to_string(),
            name,
            size,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_texture_handles_modern_and_legacy_paths() {
        assert_eq!(
            split_texture("assets/minecraft/textures/block/stone.png"),
            Some(("minecraft".to_string(), "block/stone.png".to_string()))
        );
        assert_eq!(
            split_texture("assets/custom/textures/item/gem.png"),
            Some(("custom".to_string(), "item/gem.png".to_string()))
        );
        assert_eq!(
            split_texture("textures/gui/icons.png"),
            Some(("minecraft".to_string(), "gui/icons.png".to_string()))
        );
        assert_eq!(split_texture("assets/minecraft/models/block/stone.json"), None);
        assert_eq!(split_texture("assets/minecraft/textures/foo.txt"), None);
    }

    #[test]
    fn strip_codes_removes_formatting() {
        assert_eq!(strip_codes("\u{00a7}b\u{00a7}lCool\u{00a7}r Pack"), "Cool Pack");
    }

    #[test]
    fn flatten_component_walks_text_components() {
        let value: Value = serde_json::from_str(
            r#"{"text":"Hello ","extra":[{"text":"World","color":"red"}]}"#,
        )
        .unwrap();
        let mut out = String::new();
        flatten_component(&value, &mut out);
        assert_eq!(out, "Hello World");
    }

    #[test]
    fn detect_prefix_prefers_outer_most_pack() {
        let files = vec![
            ("MyPack/pack.mcmeta".to_string(), 10),
            ("MyPack/assets/minecraft/textures/block/stone.png".to_string(), 10),
        ];
        assert_eq!(detect_prefix(&files).unwrap(), "MyPack/");

        let files = vec![
            ("pack.mcmeta".to_string(), 10),
            ("assets/minecraft/textures/block/stone.png".to_string(), 10),
        ];
        assert_eq!(detect_prefix(&files).unwrap(), "");
    }

    #[test]
    fn detect_prefix_rejects_multiple_packs() {
        let files = vec![
            ("PackA/pack.mcmeta".to_string(), 10),
            ("PackB/pack.mcmeta".to_string(), 10),
        ];
        assert!(detect_prefix(&files).is_err());
    }

    #[test]
    fn open_pack_reads_folder_manifest() {
        let root = std::env::temp_dir().join(format!("qookix-rp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("assets/minecraft/textures/block")).unwrap();
        // 清单只读 PNG 的 IHDR，24 字节头足够
        let png = |w: u32, h: u32| {
            let mut buf = b"\x89PNG\r\n\x1a\n".to_vec();
            buf.extend_from_slice(&13u32.to_be_bytes());
            buf.extend_from_slice(b"IHDR");
            buf.extend_from_slice(&w.to_be_bytes());
            buf.extend_from_slice(&h.to_be_bytes());
            buf.extend_from_slice(&[8, 6, 0, 0, 0]);
            buf
        };
        std::fs::write(root.join("assets/minecraft/textures/block/stone.png"), png(16, 16)).unwrap();
        std::fs::write(root.join("assets/minecraft/textures/block/fire_0.png"), png(16, 32)).unwrap();
        std::fs::write(
            root.join("assets/minecraft/textures/block/fire_0.png.mcmeta"),
            r#"{"animation":{"frametime":2}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("pack.mcmeta"),
            r#"{"pack":{"pack_format":15,"description":{"text":"\u00a7bCool ","extra":[{"text":"Pack"}]}}}"#,
        )
        .unwrap();

        let (manifest, _source) = open_pack(root.to_str().unwrap()).unwrap();
        assert_eq!(manifest.pack_format, Some(15));
        assert_eq!(manifest.description, "Cool Pack");
        assert_eq!(manifest.textures.len(), 2);
        assert_eq!(manifest.animated, 1);
        assert!(manifest.resolutions.contains(&16));
        assert_eq!(manifest.kind, "folder");
        let fire = manifest
            .textures
            .iter()
            .find(|t| t.file_name == "fire_0")
            .expect("应能找到动画贴图");
        assert_eq!(fire.frames, 2);
        assert_eq!(fire.category, "block");
        assert_eq!(fire.namespace, "minecraft");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_pack_reads_zip_with_nested_root() {
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!("qookix-rpz-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("CoolPack.zip");

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&32u32.to_be_bytes());
        png.extend_from_slice(&32u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);

        {
            let mut zw = zip::ZipWriter::new(File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let entries: Vec<(&str, Vec<u8>)> = vec![
                (
                    "CoolPack/pack.mcmeta",
                    br#"{"pack":{"pack_format":22,"supported_formats":{"min_inclusive":15,"max_inclusive":30},"description":"Zip Pack"}}"#.to_vec(),
                ),
                (
                    "CoolPack/assets/minecraft/textures/item/apple.png",
                    png.clone(),
                ),
                ("CoolPack/readme.txt", b"hello".to_vec()),
            ];
            for (name, data) in entries {
                zw.start_file(name, opts).unwrap();
                zw.write_all(&data).unwrap();
            }
            zw.finish().unwrap();
        }

        let (manifest, mut source) = open_pack(zip_path.to_str().unwrap()).unwrap();
        assert_eq!(manifest.kind, "zip");
        assert_eq!(manifest.pack_format, Some(22));
        assert_eq!(manifest.supported_formats.as_deref(), Some("15 – 30"));
        assert_eq!(manifest.description, "Zip Pack");
        assert_eq!(manifest.textures.len(), 1);
        assert_eq!(manifest.textures[0].category, "item");
        assert_eq!(manifest.textures[0].width, 32);

        // 前缀剥离后应能按相对路径读到贴图
        let bytes = source
            .read("assets/minecraft/textures/item/apple.png", MAX_TEXTURE_BYTES)
            .expect("应能读到嵌套目录下的贴图");
        assert!(bytes.starts_with(b"\x89PNG"));
        assert!(source.read("readme.txt", 1024).is_ok());
        // 目录穿越仍应被拒绝
        assert!(source.read("../CoolPack/pack.mcmeta", 1024).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_rel_rejects_traversal() {
        assert!(validate_rel("../secret.png").is_err());
        assert!(validate_rel("/etc/passwd").is_err());
        assert!(validate_rel("assets/x/../../y.png").is_err());
        assert!(validate_rel("C:/windows/win.ini").is_err());
        assert!(validate_rel("assets/minecraft/textures/block/stone.png").is_ok());
    }
}
