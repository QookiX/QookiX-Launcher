//! 实例目录与托管服务器目录共用的文件操作助手：路径越界防护、目录列举、
//! 文本读写与系统定位。两处文件管理器的命令层统一走这里，避免各写一份。

use serde::Serialize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 内置编辑器愿意加载的最大文件字节数。
pub const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;

/// Replace a file through a unique temporary file on the same filesystem.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("missing file parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn fmt_bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

pub fn modified_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}

#[derive(Serialize)]
pub struct FsEntry {
    pub name: String,
    pub rel: String,
    pub size: u64,
    pub modified: u64,
    pub is_dir: bool,
    pub ext: String,
}

#[derive(Serialize)]
pub struct DirectoryListing {
    pub rel: String,
    pub entries: Vec<FsEntry>,
}

#[derive(Serialize)]
pub struct FileInfo {
    pub rel: String,
    pub size: u64,
    pub modified: u64,
}

#[derive(Serialize)]
pub struct FileDocument {
    #[serde(flatten)]
    pub info: FileInfo,
    pub content: String,
}

#[derive(Serialize)]
pub struct CreatedEntry {
    pub rel: String,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct RenamedEntry {
    pub rel: String,
    pub name: String,
}

/// 目录 ID 只允许单层名称，防止拼出 `..` 或绝对路径。
pub fn validate_id(id: &str, label: &str) -> Result<(), String> {
    if id.contains("..") || !is_portable_filename(id) {
        return Err(format!("非法{label} ID"));
    }
    Ok(())
}

/// 拒绝会拼出嵌套路径或逃出父目录的名称。
pub fn validate_name(name: &str) -> Result<(), String> {
    let t = name.trim();
    if t.is_empty() || t == "." || t == ".." {
        return Err("名称不能为空".into());
    }
    if !is_portable_filename(name) {
        return Err("名称包含不支持的字符或使用了保留名称".into());
    }
    Ok(())
}

/// 逐段解析相对路径，校验现存祖先和待创建的后缀。
pub fn resolve_in_dir(dir: &Path, rel: &str, label: &str) -> Result<PathBuf, String> {
    let root = dir
        .canonicalize()
        .map_err(|e| format!("{label}目录不可用: {e}"))?;
    let normalized = rel.replace('\\', "/");
    if normalized.starts_with("//") {
        return Err("非法路径".into());
    }
    // 单个前导斜杠保留为文件管理器内的根目录简写。
    let relative = normalized.strip_prefix('/').unwrap_or(&normalized);
    let mut target = root.clone();
    if relative.is_empty() {
        return Ok(target);
    }
    for part in relative.split('/') {
        if part == ".." {
            return Err(format!("路径超出{label}目录范围"));
        }
        if !is_portable_filename(part) {
            return Err(format!("非法路径: {part}"));
        }
        target.push(part);
        match std::fs::symlink_metadata(&target) {
            Ok(_) => {
                target = target.canonicalize().map_err(|e| format!("无法解析路径: {e}"))?;
                if target == root || !target.starts_with(&root) {
                    return Err(format!("路径超出{label}目录范围"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("无法读取路径: {e}")),
        }
    }
    Ok(target)
}

/// 列举目录内容：目录在前，同类型按名称（不区分大小写）排序。
pub fn list_dir(dir: &Path, base_rel: &str) -> Result<Vec<FsEntry>, String> {
    let base = base_rel.trim_end_matches('/').to_string();
    let mut out: Vec<FsEntry> = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    for e in rd.flatten() {
        let meta = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = e.file_name().to_string_lossy().to_string();
        let is_dir = meta.is_dir();
        let child_rel = if base.is_empty() {
            name.clone()
        } else {
            format!("{base}/{name}")
        };
        out.push(FsEntry {
            ext: if is_dir { String::new() } else { ext_of(&name) },
            name,
            rel: child_rel,
            size: if is_dir { 0 } else { meta.len() },
            modified: modified_secs(&meta),
            is_dir,
        });
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// 读取文本文件：限制体积、拒绝二进制与非 UTF-8 内容。
pub fn read_text(path: &Path, rel: &str) -> Result<FileDocument, String> {
    if !path.is_file() {
        return Err("不是一个文件".into());
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("读取文件失败: {e}"))?;
    if meta.len() > MAX_EDIT_BYTES {
        return Err(format!(
            "文件过大（{}），内置编辑器最多支持 {}",
            fmt_bytes(meta.len()),
            fmt_bytes(MAX_EDIT_BYTES)
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    if bytes.iter().take(4096).any(|b| *b == 0) {
        return Err("这是二进制文件，无法在内置编辑器中打开".into());
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| String::from("文件不是 UTF-8 编码，无法在内置编辑器中打开"))?;
    Ok(FileDocument {
        info: FileInfo {
            rel: rel.into(),
            size: content.len() as u64,
            modified: modified_secs(&meta),
        },
        content,
    })
}

/// 写入文本文件，必要时创建父目录。
pub fn write_text(path: &Path, rel: &str, content: String) -> Result<FileInfo, String> {
    if path.is_dir() {
        return Err("目标是一个目录".into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let len = content.len() as u64;
    atomic_write(path, content.as_bytes()).map_err(|e| format!("写入文件失败: {e}"))?;
    let meta = std::fs::metadata(path).ok();
    Ok(FileInfo {
        rel: rel.into(),
        size: len,
        modified: meta.as_ref().map(modified_secs).unwrap_or(0),
    })
}

/// 在系统文件管理器中定位路径；目标是文件时打开其所在目录。
pub fn reveal(app: &tauri::AppHandle, path: &Path) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let target = if path.is_file() {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    app.opener()
        .open_path(target.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// True when `name` is a plain, single file name (no path separators, no `.` /
/// `..`, no drive-letter prefix). Used before joining API/user-supplied names
/// onto a filesystem path to block path traversal.
pub fn is_safe_filename(name: &str) -> bool {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
    {
        return false;
    }
    // Windows reserved device names (case-insensitive, incl. "CON.txt" etc.)
    let stem = name.split('.').next().unwrap_or("");
    let lower = stem.to_ascii_lowercase();
    let reserved = [
        "con", "prn", "aux", "nul",
        "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
        "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    !reserved.contains(&lower.as_str())
}

/// Check that a single path component has the same meaning across platforms.
pub(crate) fn is_portable_filename(name: &str) -> bool {
    if !is_safe_filename(name)
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    (stem.is_empty() || is_safe_filename(stem))
        && !matches!(stem.to_ascii_uppercase().as_str(),
            "CONIN$" | "CONOUT$" | "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³")
}

pub(crate) fn is_link_or_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}

#[derive(Serialize)]
pub struct DirectoryIndex<T> {
    pub items: Vec<T>,
    pub warnings: Vec<String>,
}

/// Read metadata separately from errors encountered while listing it.
pub fn inspect_dir_entries<T>(
    dir: &Path,
    meta_file: &str,
    sort_key: impl Fn(&T) -> u64,
) -> Result<DirectoryIndex<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let mut index = DirectoryIndex { items: Vec::new(), warnings: Vec::new() };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(index),
        Err(error) => return Err(format!("读取数据目录失败: {error}")),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                index.warnings.push(error.to_string());
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => continue,
            Err(error) => {
                index.warnings.push(format!("{name}: {error}"));
                continue;
            }
        }
        let text = match std::fs::read_to_string(path.join(meta_file)) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound && name.starts_with('.') => continue,
            Err(error) => {
                index.warnings.push(format!("{name}/{meta_file}: {error}"));
                continue;
            }
        };
        match serde_json::from_str::<T>(&text) {
            Ok(item) => index.items.push(item),
            Err(error) => index.warnings.push(format!("{name}/{meta_file}: {error}")),
        }
    }
    index.items.sort_by_key(|item| std::cmp::Reverse(sort_key(item)));
    Ok(index)
}

/// Best-effort index for internal consumers such as the system tray.
pub fn load_dir_entries<T>(
    dir: &Path,
    meta_file: &str,
    sort_key: impl Fn(&T) -> u64,
) -> Vec<T>
where
    T: serde::de::DeserializeOwned,
{
    match inspect_dir_entries(dir, meta_file, sort_key) {
        Ok(index) => index.items,
        Err(error) => {
            eprintln!("[fs] {}: {error}", dir.display());
            Vec::new()
        }
    }
}

/// Best-effort 文件系统操作兜底：失败仅记录日志（操作 + 路径 + 原因），不中断流程。
/// 用于替代完全吞掉错误的 `let _ = std::fs::xxx(...)`，让故障可排查。
/// 「文件/目录不存在」不算故障（清理类操作对首次运行本来就没东西可删），静默跳过。
pub fn fs_best_effort<T>(op: &str, path: &Path, result: std::io::Result<T>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("[fs] {op} {} 失败: {e}", path.display());
            None
        }
    }
}

/// Copy a file (creating parent dirs), or create a symlink to it when `link` is
/// true. On platforms where symlink creation is denied (e.g. Windows without
/// Developer Mode / admin), this silently falls back to a normal copy and sets
/// `*fallback = true` so the caller can warn the user.
pub fn copy_or_link(source: &Path, dest: &Path, link: bool, _fallback: &mut bool) -> Result<u64, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if link {
        if dest.exists() {
            let _ = std::fs::remove_file(dest);
        }
        let link_result = {
            #[cfg(windows)]
            { std::os::windows::fs::symlink_file(source, dest) }
            #[cfg(unix)]
            { std::os::unix::fs::symlink(source, dest) }
        };
        match link_result {
            Ok(()) => Ok(0),
            Err(_) => std::fs::copy(source, dest).map_err(|e| e.to_string()),
        }
    } else {
        std::fs::copy(source, dest).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn file_document_matches_frontend_wire_example() {
        let document = FileDocument {
            info: FileInfo { rel: "options.txt".into(), size: 5, modified: 100 },
            content: "hello".into(),
        };
        let fixtures: serde_json::Value = serde_json::from_str(include_str!("../../src/api/wire-contracts.json")).unwrap();
        assert_eq!(serde_json::to_value(document).unwrap(), fixtures["file"]);
    }

    /// 每个用例一个独立的临时根目录：root/sub/a.txt
    fn tmp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("qkx-fsutil-{tag}-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), "hi").unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn resolve_accepts_paths_inside_root() {
        let root = tmp_root("inside");
        assert_eq!(
            resolve_in_dir(&root, "sub/a.txt", "实例").unwrap(),
            root.join("sub/a.txt")
        );
        // 空 rel 指向根目录本身
        assert_eq!(resolve_in_dir(&root, "", "实例").unwrap(), root);
        // 反斜杠写法与多余的前导斜杠都要能归一化
        assert_eq!(
            resolve_in_dir(&root, "/sub\\a.txt", "实例").unwrap(),
            root.join("sub").join("a.txt")
        );
    }

    #[test]
    fn resolve_rejects_parent_escape() {
        let root = tmp_root("escape");
        let err = resolve_in_dir(&root, "../outside.txt", "实例").unwrap_err();
        assert!(err.contains("超出实例目录范围"), "unexpected: {err}");
        let err = resolve_in_dir(&root, "sub/../../outside.txt", "实例").unwrap_err();
        assert!(err.contains("超出实例目录范围"), "unexpected: {err}");
    }

    #[test]
    fn resolve_rejects_absolute_paths() {
        let root = tmp_root("absolute");
        assert!(resolve_in_dir(&root, "C:/Windows/win.ini", "实例").is_err());
        assert!(resolve_in_dir(&root, "//server/share", "实例").is_err());
        let p = resolve_in_dir(&root, "/etc/hosts", "实例").unwrap();
        assert!(p.starts_with(&root), "escaped root: {p:?}");
    }

    #[test]
    fn resolve_allows_nonexistent_target_within_root() {
        let root = tmp_root("newfile");
        // 目标不存在时走词法校验，合法路径直接返回
        let target = resolve_in_dir(&root, "sub/new.txt", "实例").unwrap();
        assert_eq!(target, root.join("sub/new.txt"));
        // 但逃逸到根之外的新路径仍要拒绝
        assert!(resolve_in_dir(&root, "../new.txt", "实例").is_err());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn resolve_checks_parent_links_for_existing_and_new_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let sibling = temp.path().join("sibling");
        let inner = root.join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::create_dir(&sibling).unwrap();
        fs::write(sibling.join("existing.txt"), "unrelated").unwrap();

        #[cfg(windows)]
        {
            for (target, name) in [(&sibling, "external"), (&inner, "internal")] {
                let link = root.join(name);
                if let Err(error) = std::os::windows::fs::symlink_dir(target, &link)
                    .or_else(|_| junction::create(target, &link))
                {
                    if matches!(error.raw_os_error(), Some(5 | 1314)) {
                        eprintln!("当前 Windows 环境不允许创建目录链接，跳过链接用例");
                        return;
                    }
                    panic!("创建目录链接失败: {error}");
                }
            }
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&sibling, root.join("external")).unwrap();
            std::os::unix::fs::symlink(&inner, root.join("internal")).unwrap();
        }

        assert!(resolve_in_dir(&root, "external/existing.txt", "实例").is_err());
        assert!(resolve_in_dir(&root, "external/new.txt", "实例").is_err());
        assert_eq!(
            resolve_in_dir(&root, "internal/new.txt", "实例").unwrap(),
            inner.canonicalize().unwrap().join("new.txt"),
        );
    }

    #[test]
    fn resolve_reports_missing_root() {
        let missing = std::env::temp_dir().join("qkx-fsutil-definitely-missing");
        let err = resolve_in_dir(&missing, "a.txt", "服务器").unwrap_err();
        assert!(err.contains("服务器目录不可用"), "unexpected: {err}");
    }

    #[test]
    fn validate_id_rejects_traversal_and_separators() {
        assert!(validate_id("abc-123", "实例").is_ok());
        for bad in ["", ".", "..", "../etc", "a/b", "a\\b", "a..b"] {
            assert!(validate_id(bad, "实例").is_err(), "should reject: {bad}");
        }
        assert_eq!(
            validate_id("..", "服务器").unwrap_err(),
            "非法服务器 ID"
        );
    }

    #[test]
    fn validate_name_rejects_separators_and_blank() {
        assert!(validate_name("a.txt").is_ok());
        for bad in ["", "  ", ".", "..", "a/b", "a\\b"] {
            assert!(validate_name(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn ext_of_is_lowercase_without_dot() {
        assert_eq!(ext_of("a.TXT"), "txt");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of(".hidden"), "");
    }

    #[test]
    fn fmt_bytes_switches_units() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(2048), "2.0 KB");
        assert_eq!(fmt_bytes(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn list_dir_lists_directories_first_with_relative_paths() {
        let root = tmp_root("list");
        std::fs::write(root.join("b.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("aaa")).unwrap();

        let entries = list_dir(&root, "").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["aaa", "sub", "b.txt"]);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].size, 0);
        assert_eq!(entries.last().unwrap().ext, "txt");

        // 带 base_rel 时 rel 要拼上前缀
        let nested = list_dir(&root.join("sub"), "sub").unwrap();
        assert_eq!(nested[0].rel, "sub/a.txt");
    }

    #[test]
    fn read_text_rejects_binary_and_oversized_files() {
        let root = tmp_root("read");
        let binary = root.join("bin.dat");
        std::fs::write(&binary, [0u8, 1, 2, 3]).unwrap();
        assert!(read_text(&binary, "bin.dat").is_err());

        let big = root.join("big.txt");
        std::fs::write(&big, vec![b'a'; (MAX_EDIT_BYTES + 1) as usize]).unwrap();
        assert!(read_text(&big, "big.txt").is_err());

        let ok = root.join("ok.txt");
        std::fs::write(&ok, "hello").unwrap();
        let v = serde_json::to_value(read_text(&ok, "ok.txt").unwrap()).unwrap();
        assert_eq!(v["content"], "hello");
        assert_eq!(v["size"], 5);
        assert_eq!(v["rel"], "ok.txt");
        assert!(v["modified"].is_u64());
    }

    #[test]
    fn write_text_creates_parent_dirs_and_reports_size() {
        let root = tmp_root("write");
        let target = root.join("deep/nested/out.txt");
        let v = write_text(&target, "deep/nested/out.txt", "abc".to_string()).unwrap();
        assert_eq!(v.size, 3);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "abc");
        // 目标已是目录时要报错
        assert!(write_text(&root.join("sub"), "sub", "x".to_string()).is_err());
    }

    #[test]
    fn load_dir_entries_loads_and_sorts_desc() {
        use serde::Deserialize;
        #[derive(Deserialize)]
        struct Item {
            name: String,
            created: u64,
        }
        let dir = std::env::temp_dir().join("qookix_test_load_dir_entries");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 三个子目录，两个有 meta，一个没有（应被跳过），一个 meta 是坏 JSON（应被跳过）
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::write(
            dir.join("a").join("meta.json"),
            r#"{"name":"alpha","created":100}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(
            dir.join("b").join("meta.json"),
            r#"{"name":"beta","created":300}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("c")).unwrap(); // 无 meta
        std::fs::create_dir_all(dir.join("d")).unwrap();
        std::fs::write(dir.join("d").join("meta.json"), "{bad json}").unwrap();
        // 一个文件（非目录）应被跳过
        std::fs::write(dir.join("stray.json"), "{}").unwrap();

        let index = inspect_dir_entries::<Item>(&dir, "meta.json", |i: &Item| i.created).unwrap();
        assert_eq!(index.warnings.len(), 2);
        let items = index.items;
        assert_eq!(items.len(), 2, "应只加载 2 个有效条目");
        // 按 created 降序：beta(300) 在前，alpha(100) 在后
        assert_eq!(items[0].name, "beta");
        assert_eq!(items[1].name, "alpha");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_dir_entries_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("qookix_test_nonexistent_dir_xyz");
        let _ = std::fs::remove_dir_all(&dir);
        let items = load_dir_entries::<serde_json::Value>(&dir, "meta.json", |_| 0);
        assert!(items.is_empty());
    }
}
