//! 地区可用性判定
//!
//! 出于合规原因，中国大陆地区的「下载并安装 Minecraft 游戏本体」相关功能
//! （原版 / Forge / Fabric / NeoForge 安装、整合包安装、镜像下载）需要停用；
//! 已经装好的实例仍可正常启动、管理。
//!
//! 判定完全离线，不联网、不上报任何信息：只读系统时区（Windows 读注册表
//! `TimeZoneInformation`，类 Unix 读 `/etc/timezone` 或 `/etc/localtime`），
//! 时区识别不出来时才退回「UTC+8 且系统语言为简体中文」。
//!
//! 结果在进程内只计算一次（`OnceLock`），运行期间不会变化。

use std::sync::OnceLock;

/// 命中限制时统一返回给前端的提示文案。
pub const BLOCKED_NOTICE: &str =
    "根据相关地区规定，中国大陆地区暂不支持在线下载与安装 Minecraft 游戏本体（含整合包）。已有实例仍可正常启动。";

static RESTRICTED: OnceLock<bool> = OnceLock::new();
static TIMEZONE_NAME: OnceLock<String> = OnceLock::new();

/// 当前是否位于中国大陆地区（进程内缓存）
pub fn is_mainland_china() -> bool {
    *RESTRICTED.get_or_init(|| {
        let tz = system_timezone();
        let _ = TIMEZONE_NAME.set(tz.clone().unwrap_or_default());
        detect(tz.as_deref())
    })
}

/// 判定用的时区名（诊断/展示用，识别不出来时为空串）
pub fn timezone_name() -> &'static str {
    TIMEZONE_NAME.get().map(String::as_str).unwrap_or("")
}

fn detect(tz: Option<&str>) -> bool {
    if let Some(tz) = tz {
        let t = tz.to_ascii_lowercase();
        // 中国大陆的 IANA 时区名与 Windows 时区名
        const HINTS: &[&str] = &[
            "asia/shanghai",
            "asia/chongqing",
            "asia/harbin",
            "asia/urumqi",
            "asia/kashgar",
            "prc",
            "china standard time",
        ];
        if HINTS.iter().any(|h| t.contains(h)) {
            return true;
        }
    }
    // 时区名拿不到（容器/精简系统）时退回：UTC+8 且简体中文
    tz.is_none() && is_utc_plus_8() && locale_is_simplified_chinese()
}

// ---------------------------------------------------------------------------
// 时区
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn system_timezone() -> Option<String> {
    windows_timezone::key_name().or_else(|| std::env::var("TZ").ok())
}

#[cfg(not(target_os = "windows"))]
fn system_timezone() -> Option<String> {
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.trim().is_empty() {
            return Some(tz);
        }
    }
    if let Ok(tz) = std::fs::read_to_string("/etc/timezone") {
        let tz = tz.trim();
        if !tz.is_empty() {
            return Some(tz.to_string());
        }
    }
    // /etc/localtime 通常是指向 .../zoneinfo/Asia/Shanghai 的符号链接
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let s = link.to_string_lossy();
        if let Some(pos) = s.find("zoneinfo/") {
            return Some(s[pos + "zoneinfo/".len()..].to_string());
        }
    }
    None
}

/// UTC 偏移是否为 +8 小时（供时区名缺失时兜底）
#[cfg(target_os = "windows")]
fn is_utc_plus_8() -> bool {
    // ActiveTimeBias 为有符号分钟数，东八区是 -480
    windows_timezone::bias_minutes().map(|b| b == -480i32).unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn is_utc_plus_8() -> bool {
    // 无标准库时区 API 时的保守做法：时区未知则不作 UTC+8 推断
    false
}

// ---------------------------------------------------------------------------
// 语言
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn locale_is_simplified_chinese() -> bool {
    windows_timezone::locale_name()
        .map(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("zh-cn") || l.starts_with("zh-hans") || l.contains("chinese (simplified)")
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn locale_is_simplified_chinese() -> bool {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .any(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("zh_cn") || v.contains("zh-cn") || v.contains("zh_hans")
        })
}

// ---------------------------------------------------------------------------
// Windows 注册表读取
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows_timezone {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_DWORD, REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 读取字符串值（REG_SZ）
    unsafe fn read_sz(hkey: HKEY, name: &str) -> Option<String> {
        let name_w = wide(name);
        let mut ty = 0u32;
        let mut size = 0u32;
        if RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null(),
            &mut ty,
            std::ptr::null_mut(),
            &mut size,
        ) != 0
            || size == 0
            || ty != REG_SZ
        {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buf.as_mut_ptr(),
            &mut size,
        ) != 0
        {
            return None;
        }
        let data = std::slice::from_raw_parts(buf.as_ptr() as *const u16, buf.len() / 2);
        let end = data.iter().position(|c| *c == 0).unwrap_or(data.len());
        Some(OsString::from_wide(&data[..end]).to_string_lossy().to_string())
    }

    /// 读取 DWORD 值（REG_DWORD）
    unsafe fn read_dword(hkey: HKEY, name: &str) -> Option<u32> {
        let name_w = wide(name);
        let mut ty = 0u32;
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        if RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null(),
            &mut ty,
            &mut data as *mut u32 as *mut u8,
            &mut size,
        ) != 0
            || ty != REG_DWORD
        {
            return None;
        }
        Some(data)
    }

    fn open(root: HKEY, path: &str) -> Option<HKEY> {
        let path_w = wide(path);
        let mut hkey: HKEY = std::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(root, path_w.as_ptr(), 0, KEY_READ, &mut hkey) };
        if rc != 0 {
            None
        } else {
            Some(hkey)
        }
    }

    /// `TimeZoneKeyName`，如 "China Standard Time"
    pub fn key_name() -> Option<String> {
        let key = open(
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation",
        )?;
        let v = unsafe { read_sz(key, "TimeZoneKeyName") };
        unsafe { RegCloseKey(key) };
        v
    }

    /// 当前生效的 UTC 偏移（分钟，西正东负）。UTC+8 => -480
    pub fn bias_minutes() -> Option<i32> {
        let key = open(
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\TimeZoneInformation",
        )?;
        let v = unsafe {
            read_dword(key, "ActiveTimeBias")
                .or_else(|| read_dword(key, "Bias"))
                .map(|v| v as i32)
        };
        unsafe { RegCloseKey(key) };
        v
    }

    /// 系统区域名，如 "zh-CN"
    pub fn locale_name() -> Option<String> {
        let key = open(HKEY_CURRENT_USER, "Control Panel\\International")?;
        let v = unsafe { read_sz(key, "LocaleName") };
        unsafe { RegCloseKey(key) };
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 打印本机判定结果，便于人工核对（用 `cargo test region -- --nocapture` 查看）
    #[test]
    fn detect_reports_timezone() {
        println!(
            "时区={:?} 受限={}",
            system_timezone(),
            is_mainland_china()
        );
    }

    /// 常见时区名的判定规则
    #[test]
    fn detect_rules() {
        assert!(detect(Some("Asia/Shanghai")));
        assert!(detect(Some("China Standard Time")));
        assert!(detect(Some("PRC")));
        assert!(!detect(Some("America/New_York")));
        assert!(!detect(Some("Europe/London")));
        assert!(!detect(Some("Asia/Tokyo")));
    }
}
