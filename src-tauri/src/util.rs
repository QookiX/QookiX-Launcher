use sha1::{Digest, Sha1};
use sha2::{Sha256, Sha512};
use std::fmt::Write as _;
use std::io::Read;
use std::path::Path;

/// Current time as Unix seconds (monotonic-ish, never panics on clock skew).
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 用任意 `Digest` 哈希器计算文件摘要（64 KiB 分块流式读取，hex 小写）。
fn file_hash<D: Digest>(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = D::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    // 注意：不能直接 format!("{:x}", hasher.finalize())——泛型 D 下
    // GenericArray 的 LowerHex 约束无法成立。write! 逐字节写入预分配的
    // String，全程零额外分配。
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}

/// sha1 of a file (hex lowercase)
pub fn file_sha1(path: &Path) -> Option<String> {
    file_hash::<Sha1>(path)
}

/// sha512 of a file (hex lowercase)
pub fn file_sha512(path: &Path) -> Option<String> {
    file_hash::<Sha512>(path)
}

/// sha256 of a file (hex lowercase)
pub fn file_sha256(path: &Path) -> Option<String> {
    file_hash::<Sha256>(path)
}

/// Read a response through the byte limit, including responses with no declared length.
pub async fn read_response_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    if response.content_length().is_some_and(|length| length > limit as u64) {
        return Err(format!("响应超过 {limit} 字节"));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| err.to_string())? {
        if chunk.len() > limit - data.len() {
            return Err(format!("响应超过 {limit} 字节"));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// 非 io::Result 版本的 best-effort 兜底（错误类型为 String 的领域函数用）。
/// 用于替代 `let _ = save_instance(...)` 这类静默吞掉的数据完整性操作。
pub fn log_best_effort<T>(what: &str, result: Result<T, String>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[best-effort] {what} 失败: {e}");
            log_line(&format!("[best-effort] {what} 失败: {e}"));
            None
        }
    }
}

/// 解码文本字节：优先 UTF-8，失败则按 GBK 解。
///
/// 为什么需要它：**老版本 Minecraft（1.8.x 这类）在中文 Windows 上把 `latest.log`
/// 按系统 ANSI 代码页（GBK/CP936）写**（Java 的默认 file.encoding），而 JVM 18+
/// 起默认才是 UTF-8。直接用 `from_utf8_lossy` 读这种日志，中文会整片变成
/// `�`（在界面上看成 `?`），而且会连带影响写进云端快照/Release 的日志片段。
///
/// 反过来说：UTF-8 文本里几乎不可能凑出合法的 GBK 字节对，所以"先试 UTF-8"是安全的。
pub fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(bytes);
            cow.into_owned()
        }
    }
}

/// 落盘诊断日志（追加写）。
/// Windows 上打包为 GUI 子系统，没有控制台，`eprintln!` 的输出用户看不到，
/// 所以排查线上问题时改为写文件：%TEMP%/qookix-install-debug.log
pub fn log_line(msg: &str) {
    let path = std::env::temp_dir().join("qookix-install-debug.log");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let line = format!("[{now}] {msg}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

/// Read a text file fully.
#[allow(dead_code)]
pub fn read_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Format a byte count for humans.
#[allow(dead_code)]
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", n, UNITS[i])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

// ---------------------------------------------------------------------------
// Mojang launch rules
// ---------------------------------------------------------------------------

pub fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "osx",
        "linux" => "linux",
        _ => "unknown",
    }
}

pub fn os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "x86" => "x86",
        "aarch64" => "arm64",
        "arm" => "arm",
        _ => "unknown",
    }
}

pub fn rule_matches(rule: &crate::models::Rule, features: &std::collections::HashMap<String, bool>) -> bool {
    if let Some(os) = &rule.os {
        if let Some(name) = &os.name {
            if name != os_name() {
                return false;
            }
        }
        if let Some(arch) = &os.arch {
            if arch != os_arch() {
                return false;
            }
        }
    }
    if let Some(fs) = &rule.features {
        for (k, v) in fs {
            // `false` means "feature not active"
            let active = features.get(k).copied().unwrap_or(false);
            if active != *v {
                return false;
            }
        }
    }
    true
}

/// Evaluate a rules list: no rules -> allowed; otherwise last match wins.
pub fn rules_allow(rules: &[crate::models::Rule], features: &std::collections::HashMap<String, bool>) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, features) {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_secs_returns_recent_unix_timestamp() {
        let t = now_secs();
        // 2024-01-01 = 1704067200, 2035-01-01 = 2051222400
        // 合理范围：不可能是 0（时钟没启动），也不可能在 2035 之后
        assert!(t > 1704067200, "now_secs 返回了过小的时间戳: {t}");
        assert!(t < 2051222400, "now_secs 返回了过大的时间戳: {t}");
    }
}
