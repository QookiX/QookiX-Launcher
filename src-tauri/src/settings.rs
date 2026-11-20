use crate::models::Settings;
use crate::state::AppState;
use directories::BaseDirs;
use std::path::{Path, PathBuf};
use sysinfo::System;

pub fn default_root() -> String {
    if let Some(base) = BaseDirs::new() {
        base.data_dir()
            .join("QookiX-Launcher")
            .to_string_lossy()
            .to_string()
    } else {
        std::env::temp_dir()
            .join("QookiX-Launcher")
            .to_string_lossy()
            .to_string()
    }
}

/// Read (total, used, available) physical memory in MB with a single refresh.
///
/// Deliberately uses `System::new()` + `refresh_memory()` instead of
/// `System::new_all()`: the latter is `RefreshKind::everything()`, which also
/// enumerates every process, CPU, network adapter, disk and user account. On a
/// cold start we called it several times, and that full sweep is exactly what
/// AV sandboxes score as "anti-VM fingerprinting" / "process enumeration"
/// (a process snapshot naturally contains `lsass.exe`). We only need the three
/// numbers from `GlobalMemoryStatusEx`, so keep the syscall surface minimal.
///
/// used = total - available, so cache that can be reclaimed is NOT counted as
/// "used" (more accurate than the old PowerShell approach).
fn memory_mb() -> Option<(u64, u64, u64)> {
    let mut sys = System::new();
    sys.refresh_memory();
    let total = sys.total_memory();
    let used = sys.used_memory();
    let available = sys.available_memory();
    if total == 0 {
        return None;
    }
    Some((total / 1024 / 1024, used / 1024 / 1024, available / 1024 / 1024))
}

/// One-shot (total, used, available) snapshot in MB.
/// Prefer this over calling the three helpers below separately: each of them
/// performs its own system query.
pub fn memory_snapshot_mb() -> Option<(u64, u64, u64)> {
    memory_mb()
}

/// Currently available physical memory in MB.
pub fn available_memory_mb() -> Option<u64> {
    memory_mb().map(|(_, _, a)| a)
}

/// Recommend (max, min) memory in MB based on available system memory and mod count.
/// - Base: 40% of available (min 2048 MB), so vanilla scales with free RAM
/// - Mods: +512 MB per 100 mods, capped at +4 GB
/// - Total capped at 75% of available (leave room for OS) and 8 GB absolute
pub fn recommended_memory(available_mb: u64, mod_count: usize) -> (u32, u32) {
    let base = (available_mb * 40 / 100).max(2048);
    let extra = (mod_count as u64 * 512 / 100).min(4096);
    let mut max = base + extra;
    let cap = (available_mb * 3 / 4).max(512);
    max = max.min(cap).min(8192).max(512);
    let min = (max / 4).max(256);
    (max as u32, min as u32)
}

pub(crate) const DATA_ROOT_POINTER: &str = "data-root.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct DataRootPointer {
    data_dir: PathBuf,
}

pub(crate) fn portable_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join("portable.txt").is_file().then(|| dir.join("data"))
}

pub(crate) fn bootstrap_root(portable: Option<PathBuf>) -> PathBuf {
    std::env::var_os("QOOKIX_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or(portable)
        .unwrap_or_else(|| PathBuf::from(default_root()))
}

fn parse_data_root(raw: &str) -> Option<PathBuf> {
    let pointer: DataRootPointer = serde_json::from_str(raw).ok()?;
    let path = pointer.data_dir;
    (path.is_absolute() && !is_disk_root(&path) && !is_system_dir(&path)).then_some(path)
}

pub fn configured_data_dir(root: &Path) -> Option<PathBuf> {
    match std::fs::read_to_string(root.join(DATA_ROOT_POINTER)) {
        Ok(raw) => match parse_data_root(&raw) {
            Some(path) => return Some(path),
            None => eprintln!("启动数据目录指针无效，尝试读取旧配置"),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("无法读取启动数据目录指针: {error}"),
    }
    // 老安装器写入 JSON 引导信息，可能比默认库里的设置更新。
    std::fs::read_to_string(root.join("settings.json"))
        .ok()
        .and_then(|raw| parse_data_root(&raw))
        .or_else(|| crate::db::existing_app_settings(root).and_then(|raw| parse_data_root(&raw)))
}

pub(crate) fn write_data_root_pointer(bootstrap: &Path, target: &Path) -> Result<(), String> {
    let pointer = DataRootPointer { data_dir: target.to_path_buf() };
    let bytes = serde_json::to_vec_pretty(&pointer).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(bootstrap).map_err(|e| format!("无法创建引导目录: {e}"))?;
    let path = bootstrap.join(DATA_ROOT_POINTER);
    crate::fsutil::atomic_write(&path, &bytes).map_err(|e| format!("无法写入数据目录指针: {e}"))?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn commit_install_data(bootstrap: &Path, data: &Path, install: &Path) -> Result<(), String> {
    if !bootstrap.is_absolute() || is_disk_root(bootstrap) || is_system_dir(bootstrap)
        || !data.is_absolute() || is_disk_root(data) || is_system_dir(data)
    {
        return Err("请选择普通的绝对数据目录".into());
    }
    if let Some(existing) = configured_data_dir(bootstrap) {
        let matches = existing == data || existing.canonicalize().ok()
            .zip(data.canonicalize().ok()).is_some_and(|(previous, target)| previous == target);
        if !matches {
            return Err(format!("已配置数据目录 {}，请使用该目录。启动器可在设置中迁移数据。", existing.display()));
        }
    }
    crate::data_root_migration::ensure_no_pending(bootstrap)?;
    let install = install.canonicalize().map_err(|error| format!("安装目录无法访问: {error}"))?;
    std::fs::create_dir_all(data).map_err(|err| format!("当前账号无法创建数据目录: {err}"))?;
    let resolved = data.canonicalize().map_err(|err| format!("无法确认实际数据目录: {err}"))?;
    if !resolved.is_dir() || is_disk_root(&resolved) || is_system_dir(&resolved)
        || resolved.starts_with(&install) || install.starts_with(&resolved)
    {
        return Err("安装目录和实际数据目录不能互相包含".into());
    }
    tempfile::NamedTempFile::new_in(&resolved).and_then(tempfile::NamedTempFile::close)
        .map_err(|err| format!("当前账号无法写入数据目录: {err}"))?;
    write_data_root_pointer(bootstrap, data)?;
    let actual = configured_data_dir(bootstrap).and_then(|path| path.canonicalize().ok());
    if actual.as_ref() != Some(&resolved) {
        return Err("数据目录引导文件写入后验证失败".into());
    }
    Ok(())
}

pub fn load_settings(root: &std::path::Path, conn: &rusqlite::Connection, fresh: bool) -> Settings {
    let stored = crate::db::get_setting(conn, "app")
        .or_else(|| std::fs::read_to_string(root.join("settings.json")).ok());
    let mut json: serde_json::Map<String, serde_json::Value> = stored.as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or_default();
    let legacy_credentials = SECRET_FIELDS.iter().any(|field| json.get(*field).is_some_and(|v| v.is_string()));
    for field in SECRET_FIELDS {
        let Some(protected) = crate::db::get_setting(conn, &credential_key(field)) else { continue };
        match crate::secret::unprotect(&protected) {
            Ok(secret) => {
                if secret.is_empty() { json.remove(field); } else { json.insert(field.into(), secret.into()); }
            }
            Err(error) => eprintln!("[credentials] 无法读取设置凭据 {field}: {error}"),
        }
    }
    let mut settings: Settings = serde_json::from_value(serde_json::Value::Object(json)).unwrap_or_default();
    // 兼容迁移：旧版本只有 proxy 没有 proxy_mode。
    // 若缺少 proxy_mode 但配置了自定义 proxy，则视为自定义模式。
    if settings.proxy_mode.is_empty() {
        settings.proxy_mode = if settings.proxy.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
            "custom".into()
        } else {
            "system".into()
        };
    }
    // 兼容迁移：旧版本的「关闭窗口时」默认值是 "minimize"，新默认值是「每次询问」。
    // 用户从未明确选择过（close_behavior_prompted 为 false）时迁移为 "ask"，
    // 让老用户也能用上新默认；显式选择过则保留原值。
    let update_close_behavior = !settings.close_behavior_prompted && settings.close_behavior == "minimize";
    if update_close_behavior {
        settings.close_behavior = "ask".into();
    }
    settings.data_dir = root.to_string_lossy().into_owned();
    if (legacy_credentials || update_close_behavior) && crate::db::require_persistent(conn).is_ok() {
        if let Err(error) = save_settings(conn, &settings, None) {
            eprintln!("[credentials] 无法迁移启动器设置: {error}");
        } else if legacy_credentials {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }
    if crate::db::get_setting(conn, "app").is_some() {
        scrub_legacy_settings_copies(root, conn);
    }
    // First launch: auto-detect memory with a single snapshot (previously two
    // separate full-system refreshes).
    if fresh && stored.is_none() {
        if let Some((_, _, available)) = memory_mb() {
            let (max, min) = recommended_memory(available, 0);
            settings.max_memory_mb = max;
            settings.min_memory_mb = min;
        }
    }
    settings
}

/// 由系统密钥保护的设置字段。
///
/// 命令返回值在前端 DevTools 里一条 `await api.getSettings()` 就能拿到，
/// 所以 API Key 这类东西绝不能出现在 `get_settings` / `set_settings` 的返回体里。
const SECRET_FIELDS: [&str; 4] = [
    "curseforge_api_key",
    "translate_api_key",
    "shimu_client_secret",
    "littleskin_client_secret",
];

fn credential_key(field: &str) -> String {
    format!("credentials.{field}")
}

fn save_settings(conn: &rusqlite::Connection, settings: &Settings, patch: Option<&serde_json::Value>) -> Result<(), String> {
    let mut json = serde_json::to_value(settings).map_err(|err| err.to_string())?;
    let settings = json.as_object_mut().ok_or("启动器设置格式无效")?;
    let tx = conn.unchecked_transaction().map_err(|err| err.to_string())?;
    for field in SECRET_FIELDS {
        let value = settings.remove(field);
        let key = credential_key(field);
        let explicit = patch.and_then(|patch| patch.get(field)).is_some_and(|value| value.is_string());
        if explicit || crate::db::get_setting(&tx, &key).is_none() {
            if let Some(value) = value.as_ref().and_then(|v| v.as_str()).filter(|v| !v.is_empty()) {
                crate::db::set_setting(&tx, &key, &crate::secret::protect(value)?)?;
            } else if explicit {
                crate::db::set_setting(&tx, &key, "")?;
            }
        }
    }
    crate::db::set_setting(&tx, "app", &json.to_string())?;
    tx.commit().map_err(|err| err.to_string())
}

pub(crate) fn import_legacy_settings(conn: &rusqlite::Connection, mut settings: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    for field in SECRET_FIELDS {
        if let Some(value) = settings.remove(field).and_then(|v| v.as_str().map(str::to_owned)).filter(|v| !v.is_empty()) {
            crate::db::set_setting(conn, &credential_key(field), &crate::secret::protect(&value)?)?;
        }
    }
    crate::db::set_setting(conn, "app", &serde_json::Value::Object(settings).to_string())
}

pub(crate) fn scrub_legacy_settings_copy(conn: &rusqlite::Connection, path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    let mut json: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
    let mut changed = false;
    for field in SECRET_FIELDS {
        let Some(value) = json.get(field).and_then(|v| v.as_str()) else { continue };
        let persisted = crate::db::get_setting(conn, &credential_key(field))
            .and_then(|secret| crate::secret::unprotect(&secret).ok());
        if persisted.as_deref() == Some(value) {
            json.remove(field);
            changed = true;
        }
    }
    if changed {
        let sanitized = serde_json::to_vec_pretty(&json).map_err(|err| err.to_string())?;
        crate::fsutil::atomic_write(path, &sanitized).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn scrub_legacy_settings_copies(root: &Path, conn: &rusqlite::Connection) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let known_backup = name.strip_prefix("settings.json.migrated-").and_then(|name| name.strip_suffix(".bak"))
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok());
        if (matches!(name, "settings.json" | "settings.json.migrated.bak") || known_backup)
            && entry.file_type().is_ok_and(|file| file.is_file())
        {
            if let Err(error) = scrub_legacy_settings_copy(conn, &entry.path()) {
                eprintln!("[credentials] 旧设置副本整理失败: {error}");
            }
        }
    }
}

/// 给前端的设置视图：敏感字段替换成「是否已配置 + 掩码尾号」。
///
/// 用 JSON 投影而不是另写一个几十字段的结构体：`Settings` 以后加字段不会漏同步
/// （`frontend_view_strips_secrets` 测试会在敏感字段漏掉时报警）。
pub fn frontend_view(settings: &Settings) -> serde_json::Value {
    let mut view = serde_json::to_value(settings).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = view.as_object_mut() {
        for field in SECRET_FIELDS {
            let raw = obj
                .remove(field)
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            obj.insert(
                format!("{field}_set"),
                serde_json::Value::Bool(!raw.trim().is_empty()),
            );
            obj.insert(
                format!("{field}_hint"),
                serde_json::Value::String(mask_secret(&raw)),
            );
        }
    }
    view
}

/// 只留末尾 4 位（`····ab12`）：够用户认出配的是哪把 Key，又不足以还原。
/// 不足 8 位时连尾号也不给（短 Key 露出 4 位等于泄露一半）。
fn mask_secret(key: &str) -> String {
    let key = key.trim();
    let len = key.chars().count();
    if len == 0 {
        return String::new();
    }
    if len < 8 {
        return "····".to_string();
    }
    let tail: String = key.chars().skip(len - 4).collect();
    format!("····{tail}")
}

/// 处理前端回传的敏感字段：只接受字符串（空串 = 用户主动清空）。
///
/// 前端拿不到已保存的 Key，设置页「保存」会把整份设置对象发回来，此时这两个
/// 字段是缺失的；旧代码把 null 也当清空，那种写法在字段缺失时会把 Key 抹掉。
fn set_secret(slot: &mut Option<String>, value: &serde_json::Value) {
    if let Some(s) = value.as_str() {
        let s = s.trim();
        *slot = if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        };
    }
}

/// Update settings with a partial patch (values from frontend).
pub fn update_settings(state: &AppState, patch: serde_json::Value) -> Result<Settings, String> {
    let conn = state.db();
    crate::db::require_persistent(&conn)?;
    let mut current = state.settings_mut();
    let mut settings = current.clone();
    // NOTE: `data_dir` is intentionally ignored here. The data root is fixed at
    // startup (honoring the value the installer seeds into settings.json) and
    // existing data is not migrated; applying a runtime change would silently
    // make the app "lose" everything on the next launch.
    if let Some(v) = patch.get("java_path") {
        settings.java_path = v.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty());
    }
    if let Some(v) = patch.get("max_memory_mb").and_then(|v| v.as_u64()) {
        settings.max_memory_mb = v as u32;
    }
    if let Some(v) = patch.get("min_memory_mb").and_then(|v| v.as_u64()) {
        settings.min_memory_mb = v as u32;
    }
    if let Some(v) = patch.get("memory_mode").and_then(|v| v.as_str()) {
        settings.memory_mode = v.to_string();
    }
    if let Some(v) = patch.get("jvm_args").and_then(|v| v.as_str()) {
        settings.jvm_args = v.to_string();
    }
    if let Some(v) = patch.get("game_args").and_then(|v| v.as_str()) {
        settings.game_args = v.to_string();
    }
    if let Some(v) = patch.get("download_threads").and_then(|v| v.as_u64()) {
        settings.download_threads = (v as usize).clamp(1, 64);
    }
    if let Some(v) = patch.get("download_chunk_threads").and_then(|v| v.as_u64()) {
        settings.download_chunk_threads = (v as usize).clamp(1, 16);
    }
    if let Some(v) = patch.get("curseforge_api_key") {
        set_secret(&mut settings.curseforge_api_key, v);
    }
    if let Some(v) = patch.get("theme").and_then(|v| v.as_str()) {
        settings.theme = v.to_string();
    }
    if let Some(v) = patch.get("theme_color").and_then(|v| v.as_str()) {
        if !v.trim().is_empty() {
            settings.theme_color = v.trim().to_string();
        }
    }
    if let Some(v) = patch.get("language").and_then(|v| v.as_str()) {
        let l = v.trim();
        if matches!(l, "zh-CN" | "en-US") {
            settings.language = l.to_string();
        }
    }
    if let Some(v) = patch.get("close_behavior").and_then(|v| v.as_str()) {
        let b = v.trim();
        if matches!(b, "ask" | "minimize" | "quit") {
            settings.close_behavior = b.to_string();
            // 设置页里的点选视为用户的明确选择，之后不再被迁移覆盖
            settings.close_behavior_prompted = true;
        }
    }
    if let Some(v) = patch.get("auto_launch").and_then(|v| v.as_bool()) {
        settings.auto_launch = v;
    }
    if let Some(v) = patch.get("keep_open").and_then(|v| v.as_bool()) {
        settings.keep_open = v;
    }
    if let Some(v) = patch.get("ms_client_id").and_then(|v| v.as_str()) {
        if !v.trim().is_empty() {
            settings.ms_client_id = v.trim().to_string();
        }
    }
    // 拾木空间 OAuth 凭证：client_id 非敏感可直接回传，client_secret 走 set_secret
    if let Some(v) = patch.get("shimu_client_id").and_then(|v| v.as_str()) {
        settings.shimu_client_id = v.trim().to_string();
    }
    if let Some(v) = patch.get("shimu_client_secret") {
        set_secret(&mut settings.shimu_client_secret, v);
    }
    // LittleSkin OAuth2 凭证：client_id 非敏感可直接回传，client_secret 走 set_secret
    if let Some(v) = patch.get("littleskin_client_id").and_then(|v| v.as_str()) {
        settings.littleskin_client_id = v.trim().to_string();
    }
    if let Some(v) = patch.get("littleskin_client_secret") {
        set_secret(&mut settings.littleskin_client_secret, v);
    }
    if let Some(v) = patch.get("selected_account") {
        settings.selected_account = v.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty());
    }
    if let Some(v) = patch.get("proxy_mode").and_then(|v| v.as_str()) {
        let m = v.trim().to_string();
        if matches!(m.as_str(), "system" | "direct" | "custom") {
            settings.proxy_mode = m;
        }
    }
    if let Some(v) = patch.get("proxy") {
        let p = v.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        settings.proxy = p;
    }
    if let Some(v) = patch.get("mirror").and_then(|v| v.as_str()) {
        settings.mirror = v.trim().to_string();
    }
    if let Some(v) = patch.get("mirror_custom").and_then(|v| v.as_str()) {
        settings.mirror_custom = v.trim().to_string();
    }
    // 内容 API 镜像：只接受内置预设 id 或 "custom"，其它值一律回落到官方源，
    // 避免前端传来脏值后把发现页的请求地址拼成不可用的 URL。
    if let Some(v) = patch.get("content_mirror").and_then(|v| v.as_str()) {
        let id = v.trim();
        let known = id == crate::mirror::CONTENT_CUSTOM
            || crate::mirror::CONTENT_PRESETS.iter().any(|p| p.id == id);
        settings.content_mirror = if known {
            id.to_string()
        } else {
            crate::mirror::CONTENT_OFFICIAL.to_string()
        };
    }
    if let Some(v) = patch.get("content_mirror_custom").and_then(|v| v.as_str()) {
        settings.content_mirror_custom = v.trim().trim_end_matches('/').to_string();
    }
    // 背景类型白名单：未知值一律归零（= 自动：有图用图，无图渐变），
    // 避免脏配置把前端背景切成一片空白。
    if let Some(v) = patch.get("background_mode") {
        settings.background_mode = match v.as_str().unwrap_or("") {
            m @ ("gradient" | "image" | "scene") => m.to_string(),
            _ => String::new(),
        };
    }
    // 动态场景 id 白名单：未知值归零（= 默认场景），避免脏配置把背景切成空白
    if let Some(v) = patch.get("background_scene") {
        settings.background_scene = match v.as_str().unwrap_or("") {
            m @ ("island" | "train") => m.to_string(),
            _ => String::new(),
        };
    }
    // 动态场景画质白名单：未知值归零（= 默认档 medium）
    if let Some(v) = patch.get("background_quality") {
        settings.background_quality = match v.as_str().unwrap_or("") {
            q @ ("low" | "medium" | "high") => q.to_string(),
            _ => String::new(),
        };
    }
    if let Some(v) = patch.get("background_image") {
        settings.background_image = v.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty());
    }
    if let Some(v) = patch.get("background_blur").and_then(|v| v.as_u64()) {
        settings.background_blur = (v as u32).min(50);
    }
    if let Some(v) = patch.get("background_dim").and_then(|v| v.as_u64()) {
        settings.background_dim = (v as u32).min(100);
    }
    if let Some(v) = patch.get("glass_blur").and_then(|v| v.as_u64()) {
        settings.glass_blur = (v as u32).min(30);
    }
    if let Some(v) = patch.get("show_news").and_then(|v| v.as_bool()) {
        settings.show_news = v;
    }
    if let Some(arr) = patch.get("hidden_nav_items").and_then(|v| v.as_array()) {
        settings.hidden_nav_items = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(v) = patch.get("dismissed_update_version") {
        settings.dismissed_update_version =
            v.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty());
    }
    if let Some(v) = patch.get("auto_update").and_then(|v| v.as_bool()) {
        settings.auto_update = v;
    }
    if let Some(v) = patch.get("update_source").and_then(|v| v.as_str()) {
        settings.update_source = if v.trim() == "github" {
            "github".into()
        } else {
            "bucket".into()
        };
    }
    if let Some(v) = patch.get("translate_provider").and_then(|v| v.as_str()) {
        settings.translate_provider = match v.trim() {
            "custom" => "custom".into(),
            "baidu_web" => "baidu_web".into(),
            _ => "default".into(),
        };
    }
    if let Some(v) = patch.get("translate_api_base").and_then(|v| v.as_str()) {
        settings.translate_api_base = v.trim().trim_end_matches('/').to_string();
    }
    if let Some(v) = patch.get("translate_api_key") {
        set_secret(&mut settings.translate_api_key, v);
    }
    if let Some(v) = patch.get("translate_api_model").and_then(|v| v.as_str()) {
        settings.translate_api_model = v.trim().to_string();
    }
    if let Some(v) = patch.get("onboarding_completed").and_then(|v| v.as_bool()) {
        settings.onboarding_completed = v;
    }
    if let Some(v) = patch.get("body_translate_auto").and_then(|v| v.as_bool()) {
        settings.body_translate_auto = v;
    }
    if let Some(v) = patch.get("share_activity").and_then(|v| v.as_bool()) {
        settings.share_activity = v;
    }
    save_settings(&conn, &settings, Some(&patch))?;
    *current = settings.clone();
    Ok(settings)
}

/// Check if a path is a filesystem root (e.g. `C:\` on Windows, `/` on Unix).
/// Uses path components so it works for both raw and canonicalized (UNC) paths.
pub fn is_disk_root(path: &std::path::Path) -> bool {
    use std::path::Component;
    path.components()
        .all(|c| matches!(c, Component::RootDir | Component::Prefix(_)))
}

/// Check if a path is a sensitive system directory that must not be used as
/// the launcher data root (would mix launcher files with OS files).
pub fn is_system_dir(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase();
    #[cfg(windows)]
    {
        let s = s.trim_end_matches('\\');
        if s.ends_with("\\windows") || s.ends_with("\\program files")
            || s.ends_with("\\program files (x86)") || s.ends_with("\\programdata")
            || s.ends_with("\\system32") || s.ends_with("\\$recycle.bin")
        {
            return true;
        }
    }
    #[cfg(unix)]
    {
        if matches!(
            s.as_str(),
            "/etc" | "/var" | "/usr" | "/bin" | "/sbin" | "/boot" | "/sys" | "/proc"
        ) {
            return true;
        }
    }
    false
}

/// Publish a pointer immediately or schedule an offline directory migration for restart.
pub fn change_data_dir(state: &AppState, new_dir: &str, mode: &str) -> Result<String, String> {
    let root = Path::new(new_dir);
    if !root.is_absolute() || is_disk_root(root) || is_system_dir(root) {
        return Err("请选择绝对路径下的普通目录".into());
    }
    let target = root.canonicalize().map_err(|error| format!("无法找到目标数据目录: {error}"))?;
    if !target.is_dir() || is_disk_root(&target) || is_system_dir(&target) {
        return Err("请选择普通的数据目录".into());
    }
    if state.root.canonicalize().as_ref().ok() == Some(&target) {
        return Err("新目录与当前数据目录相同".into());
    }

    let bootstrap = bootstrap_root(portable_data_dir());
    let normalized = new_dir.trim_end_matches(['\\', '/']).to_string();
    match mode {
        "pointer" => {
            state.jobs.queue_relocation(|| {
                crate::data_root_migration::ensure_no_pending(&bootstrap)?;
                write_data_root_pointer(&bootstrap, Path::new(&normalized))
            })?;
            state.settings_mut().data_dir = normalized.clone();
        }
        "copy" | "move" => {
            use crate::data_root_migration::{self, MigrationMode};
            let mode = if mode == "move" { MigrationMode::Move } else { MigrationMode::Copy };
            state.jobs.queue_relocation(|| data_root_migration::schedule(&bootstrap, &state.root, &target, mode))?;
        }
        other => return Err(format!("未知迁移模式: {other}")),
    }
    Ok(normalized)
}

/// Ensure the directory layout exists under the given root.
pub fn ensure_layout(root: &std::path::Path) -> std::io::Result<()> {
    for sub in [
        "instances",
        "libraries",
        "assets/indexes",
        "assets/objects",
        "versions",
        "logs",
        "runtimes",
        "skins",
    ] {
        std::fs::create_dir_all(root.join(sub))?;
    }
    Ok(())
}

/// Shared http client builder with a browser-ish UA.
pub fn http_client(proxy_mode: &str, proxy: Option<&str>) -> reqwest::Client {
    http_client_timed(
        proxy_mode,
        proxy,
        std::time::Duration::from_secs(15),
        std::time::Duration::from_secs(60),
    )
}

/// 与 [`http_client`] 使用完全相同的 UA / 代理配置，但可指定超时。
///
/// 供"探测类"请求使用：既要走用户配置的代理（否则配了代理也照样直连失败），
/// 又要用短超时让坏源尽快失败、不拖住启动流程。
pub fn http_client_timed(
    proxy_mode: &str,
    proxy: Option<&str>,
    connect_timeout: std::time::Duration,
    total_timeout: std::time::Duration,
) -> reqwest::Client {
    build_http_client(proxy_mode, proxy, connect_timeout, total_timeout, true)
}

pub(crate) fn oauth_http_client(state: &AppState) -> reqwest::Client {
    let settings = state.settings();
    build_http_client(&settings.proxy_mode, settings.proxy.as_deref(),
        std::time::Duration::from_secs(12), std::time::Duration::from_secs(25), false)
}

fn build_http_client(
    proxy_mode: &str,
    proxy: Option<&str>,
    connect_timeout: std::time::Duration,
    total_timeout: std::time::Duration,
    follow_redirects: bool,
) -> reqwest::Client {
    let user_agent = format!("QookiX-Launcher/{} (desktop)", env!("CARGO_PKG_VERSION"));
    let base = || {
        let builder = reqwest::Client::builder()
            .user_agent(&user_agent)
            .gzip(true)
            .brotli(true)
            .connect_timeout(connect_timeout)
            .timeout(total_timeout);
        if follow_redirects { builder } else { builder.redirect(reqwest::redirect::Policy::none()) }
    };
    let mut builder = base();
    match proxy_mode {
        // 直连：禁用所有代理（含系统代理）
        "direct" => {
            builder = builder.no_proxy();
        }
        // 自定义：使用显式配置的代理 URL
        "custom" => {
            if let Some(p) = proxy {
                if !p.is_empty() {
                    if let Ok(proxy) = reqwest::Proxy::all(p) {
                        builder = builder.proxy(proxy);
                    }
                }
            }
        }
        // "system" 及其它：不额外设置，走 reqwest 默认的系统代理
        _ => {}
    }
    // 构建失败不再 panic（旧实现 expect 会让应用在启动路径直接崩溃，
    // 典型诱因是用户配置的自定义代理），回退为不带自定义代理的默认客户端。
    // 无自定义代理的兜底构建在实际中不会失败。
    match builder.build() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[settings] HTTP 客户端构建失败({e})，已回退为默认网络配置");
            base().build().expect("default http client build")
        }
    }
}

#[allow(dead_code)]
pub fn state_guard<'a>(state: &'a AppState) -> std::sync::RwLockReadGuard<'a, Settings> {
    state.settings()
}

#[allow(dead_code)]
pub fn with_settings<T>(state: &AppState, f: impl FnOnce(&Settings) -> T) -> T {
    let s = state.settings();
    f(&s)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn installer_commits_and_rereads_the_actual_user_data_pointer() {
        let root = tempfile::tempdir().unwrap();
        let (bootstrap, data, install) = (root.path().join("bootstrap"), root.path().join("data"), root.path().join("app"));
        std::fs::create_dir(&install).unwrap();
        commit_install_data(&bootstrap, &data, &install).unwrap();
        assert!(data.is_dir());
        assert_eq!(configured_data_dir(&bootstrap).unwrap(), data);
        assert!(commit_install_data(&bootstrap, &root.path().join("other"), &install).is_err());
        assert_eq!(configured_data_dir(&bootstrap).unwrap(), data);
    }
    use super::*;
    use serde_json::json;

    #[test]
    fn separate_pointer_beats_stale_sqlite_and_leaves_target_contents_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let bootstrap = temp.path().join("boot");
        let target = temp.path().join("target");
        std::fs::create_dir_all(&bootstrap).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("sentinel"), "keep").unwrap();

        let conn = crate::db::open(&bootstrap).unwrap();
        let stale = temp.path().join("outdated");
        save_settings(&conn, &Settings { data_dir: stale.to_string_lossy().into_owned(), ..Settings::default() }, None).unwrap();
        assert_eq!(configured_data_dir(&bootstrap), Some(stale));

        write_data_root_pointer(&bootstrap, &target).unwrap();
        assert_eq!(configured_data_dir(&bootstrap), Some(target.clone()));
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
        assert_eq!(std::fs::read_to_string(target.join("sentinel")).unwrap(), "keep");
    }

    #[test]
    fn pointer_write_reports_a_failed_publish() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(DATA_ROOT_POINTER)).unwrap();
        assert!(write_data_root_pointer(temp.path(), temp.path()).is_err());
        assert!(temp.path().join(DATA_ROOT_POINTER).is_dir());
    }

    /// 下发给前端的设置里不能带任何 Key，但要告诉前端「配没配」和尾号
    #[test]
    fn frontend_view_strips_secrets() {
        let s = Settings {
            curseforge_api_key: Some("cf-secret-key-1234".into()),
            translate_api_key: Some("sk-translate-9876".into()),
            theme: "light".into(),
            ..Default::default()
        };

        let view = frontend_view(&s);
        let text = serde_json::to_string(&view).unwrap();
        assert!(
            !text.contains("cf-secret-key-1234") && !text.contains("sk-translate-9876"),
            "返回体里不能出现 Key 原文：{text}"
        );
        assert!(view.get("curseforge_api_key").is_none());
        assert!(view.get("translate_api_key").is_none());

        assert_eq!(view["curseforge_api_key_set"], json!(true));
        assert_eq!(view["translate_api_key_set"], json!(true));
        assert_eq!(view["curseforge_api_key_hint"], json!("····1234"));
        assert_eq!(view["translate_api_key_hint"], json!("····9876"));
        // 其它字段要照常透传（这条防的是「投影把字段写漏」）
        assert_eq!(view["theme"], json!("light"));
        assert_eq!(view["max_memory_mb"], json!(s.max_memory_mb));
        assert_eq!(view["language"], json!(s.language));
    }

    #[test]
    fn frontend_view_flags_missing_keys() {
        // curseforge 留默认 None；translate 只填空白 → 两者都算未配置
        let s = Settings {
            translate_api_key: Some("   ".into()),
            ..Default::default()
        };

        let view = frontend_view(&s);
        assert_eq!(view["curseforge_api_key_set"], json!(false));
        assert_eq!(view["translate_api_key_set"], json!(false));
        assert_eq!(view["curseforge_api_key_hint"], json!(""));
        assert_eq!(view["translate_api_key_hint"], json!(""));
    }

    #[test]
    fn mask_keeps_only_tail_of_long_keys() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("   "), "");
        assert_eq!(mask_secret("short"), "····");
        assert_eq!(mask_secret("12345678"), "····5678");
        assert_eq!(mask_secret("  sk-abcdefgh  "), "····efgh");
    }

    /// 前端整份设置回传时（字段缺失 / null）不能把已存的 Key 抹掉
    #[test]
    fn set_secret_keeps_value_on_null() {
        let mut slot = Some("old".to_string());
        set_secret(&mut slot, &json!(null));
        assert_eq!(slot.as_deref(), Some("old"), "null 不能抹掉已存的 Key");

        set_secret(&mut slot, &json!("  new-key  "));
        assert_eq!(slot.as_deref(), Some("new-key"));

        set_secret(&mut slot, &json!(""));
        assert_eq!(slot, None, "空串是用户主动清空");
    }

    #[test]
    fn is_disk_root_detects_drive_root() {
        assert!(is_disk_root(std::path::Path::new("/")));
        #[cfg(windows)]
        {
            assert!(is_disk_root(std::path::Path::new("C:\\")));
            assert!(is_disk_root(std::path::Path::new("D:\\")));
        }
    }

    #[test]
    fn is_disk_root_rejects_normal_dirs() {
        assert!(!is_disk_root(std::path::Path::new("/home/user")));
        #[cfg(windows)]
        {
            assert!(!is_disk_root(std::path::Path::new("C:\\QookiX")));
            assert!(!is_disk_root(std::path::Path::new("D:\\Games\\MC")));
        }
    }

    #[cfg(windows)]
    #[test]
    fn is_disk_root_handles_unc_canonicalized() {
        // canonicalize("C:\\") returns \\?\C:\ — must still be detected as root
        let p = std::path::Path::new("C:\\").canonicalize().unwrap();
        assert!(is_disk_root(&p));
    }

    #[cfg(windows)]
    #[test]
    fn is_system_dir_detects_windows_dirs() {
        assert!(is_system_dir(std::path::Path::new("C:\\Windows")));
        assert!(is_system_dir(std::path::Path::new("C:\\Program Files")));
        assert!(is_system_dir(std::path::Path::new("C:\\Program Files (x86)")));
        assert!(is_system_dir(std::path::Path::new("C:\\ProgramData")));
        assert!(!is_system_dir(std::path::Path::new("D:\\Games")));
    }
}
    #[test]
    fn saved_credentials_are_separate_and_legacy_copy_is_sanitized() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let key = "fixture-key-1234";
        let original = serde_json::json!({ "language": "en-US", "curseforge_api_key": key });
        std::fs::write(root.join("settings.json"), original.to_string()).unwrap();
        let conn = crate::db::initialize(root).unwrap();
        let app = crate::db::get_setting(&conn, "app").unwrap();
        assert!(!app.contains(key));
        let protected = crate::db::get_setting(&conn, &credential_key("curseforge_api_key")).unwrap();
        assert_ne!(protected, key);
        assert_eq!(crate::secret::unprotect(&protected).unwrap(), key);
        assert!(!std::fs::read_to_string(root.join("settings.json.migrated.bak")).unwrap().contains(key));
        let settings = load_settings(root, &conn, false);
        assert_eq!(settings.curseforge_api_key.as_deref(), Some(key));
        let reset = Settings { curseforge_api_key: None, ..settings };
        save_settings(&conn, &reset, Some(&serde_json::json!({ "curseforge_api_key": "" }))).unwrap();
        assert!(load_settings(root, &conn, false).curseforge_api_key.is_none());
    }
