use crate::models::JavaInfo;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

mod runtime;
pub use runtime::download_java_runtime;
pub(crate) use runtime::download_cache_dir;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const SCAN_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_OUTPUT_LIMIT: u64 = 1024 * 1024;
const MAX_CANDIDATES: usize = 64;
const MAX_SCAN_ENTRIES: usize = 16_384;

/// Hide the console window when spawning console-subsystem children
/// (`reg.exe`, `java.exe`) on Windows; otherwise each child flashes a cmd
/// window open-and-closed from a GUI process.
#[cfg(windows)]
fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn hidden(cmd: &mut Command) -> &mut Command {
    cmd
}

fn probe_output(command: &mut Command, deadline: Instant) -> std::io::Result<Output> {
    let timed_out = || std::io::Error::new(std::io::ErrorKind::TimedOut, "Java probe timed out");
    if Instant::now() >= deadline {
        return Err(timed_out());
    }
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut child = hidden(command)
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .spawn()?;
    let status = loop {
        let result = (|| {
            let exited = child.try_wait()?;
            if stdout.metadata()?.len() > PROBE_OUTPUT_LIMIT || stderr.metadata()?.len() > PROBE_OUTPUT_LIMIT {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Java probe output too large"));
            }
            match exited {
                Some(status) => Ok(Some(status)),
                None if Instant::now() >= deadline => Err(timed_out()),
                None => Ok(None),
            }
        })();
        match result {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        }
    };
    let read_output = |file: &mut std::fs::File| -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.take(PROBE_OUTPUT_LIMIT).read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    Ok(Output { status, stdout: read_output(&mut stdout)?, stderr: read_output(&mut stderr)? })
}

struct ScanBudget {
    deadline: Instant,
    remaining_entries: usize,
}

impl ScanBudget {
    fn visit(&mut self) -> bool {
        if self.remaining_entries == 0 || Instant::now() >= self.deadline {
            return false;
        }
        self.remaining_entries -= 1;
        true
    }
}

// ---------------------------------------------------------------------------
// Persisted detection cache
// ---------------------------------------------------------------------------
// Java detection runs 26+ `reg query` calls, so we persist the result to disk
// and only rescan when it goes stale (or the user forces a refresh, or a new
// runtime is downloaded). This avoids scanning on every app launch.
const JAVA_CACHE_FILE: &str = "java-cache.json";
const JAVA_CACHE_TTL_SECS: u64 = 30 * 24 * 60 * 60; // 30 days

#[derive(serde::Serialize, serde::Deserialize)]
struct JavaCache {
    fetched_at: u64,
    candidates: Vec<JavaInfo>,
}

fn load_java_cache(root: &Path) -> Option<(u64, Vec<JavaInfo>)> {
    let text = std::fs::read_to_string(root.join(JAVA_CACHE_FILE)).ok()?;
    let cache: JavaCache = serde_json::from_str(&text).ok()?;
    Some((cache.fetched_at, cache.candidates))
}

fn save_java_cache(root: &Path, candidates: &[JavaInfo], now: u64) {
    let cache = JavaCache {
        fetched_at: now,
        candidates: candidates.to_vec(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = std::fs::write(root.join(JAVA_CACHE_FILE), json);
    }
}

fn invalidate_java_cache(root: &Path) {
    let _ = std::fs::remove_file(root.join(JAVA_CACHE_FILE));
}

/// Detected Java list, reusing the persisted cache when it is fresh enough;
/// otherwise scan, persist the result, and return it. `force` skips the disk
/// cache entirely (used by the manual “查找 Java” refresh).
pub fn cached_detect(
    root: &Path,
    runtimes: &Path,
    now: u64,
    force: bool,
) -> (u64, Vec<JavaInfo>) {
    detect_with_cache(root, now, force, || detect_java(None, Some(runtimes)))
}

fn detect_with_cache(
    root: &Path,
    now: u64,
    force: bool,
    detect: impl FnOnce() -> Vec<JavaInfo>,
) -> (u64, Vec<JavaInfo>) {
    if !force {
        if let Some((ts, list)) = load_java_cache(root) {
            if now.saturating_sub(ts) < JAVA_CACHE_TTL_SECS {
                return (now, list);
            }
        }
    }
    let detected = detect();
    save_java_cache(root, &detected, now);
    (now, detected)
}

/// Detect Java installations on this machine, including any runtimes the
/// launcher itself downloaded under `runtime_root`.
///
/// Sources:
///  - user-selected path from settings
///  - `JAVA_HOME` / `JRE_HOME` env
///  - `java` / `javaw` on PATH
///  - Windows registry (HKLM/HKCU `Software\JavaSoft\...` and Adoptium)
///  - common install directories
///  - launcher-managed runtimes under `<root>/runtimes/java`
pub fn detect_java(custom: Option<&str>, runtime_root: Option<&Path>) -> Vec<JavaInfo> {
    let deadline = Instant::now() + SCAN_TIMEOUT;
    let mut budget = ScanBudget {
        deadline: deadline - Duration::from_secs(8),
        remaining_entries: MAX_SCAN_ENTRIES,
    };
    let mut paths: Vec<PathBuf> = Vec::new();

    if let Some(c) = custom {
        if !c.trim().is_empty() {
            paths.push(PathBuf::from(c.trim()));
        }
    }
    for var in ["JAVA_HOME", "JRE_HOME", "JDK_HOME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                paths.push(PathBuf::from(v).join("bin").join(java_exe()));
            }
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !budget.visit() {
                break;
            }
            let cand = dir.join(java_exe());
            if cand.is_file() {
                paths.push(cand);
            }
        }
    }
    paths.extend(runtime_java_paths(runtime_root, &mut budget));
    paths.extend(registry_java_paths(budget.deadline));
    paths.extend(fs_scan_paths(&mut budget));

    // de-dupe preserving order (also collapse java.exe / javaw.exe pairs
    // from the same bin directory)
    let mut seen = std::collections::HashSet::new();
    let mut seen_dirs = std::collections::HashSet::new();
    let mut out: Vec<JavaInfo> = Vec::new();
    let mut probed = 0;
    for p in paths {
        if Instant::now() >= deadline || probed == MAX_CANDIDATES {
            break;
        }
        let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        let key = path_key(&canon);
        if !seen.insert(key) {
            continue;
        }
        let dir = path_key(canon.parent().unwrap_or(&canon));
        if seen_dirs.contains(&dir) || !canon.is_file() {
            continue;
        }
        probed += 1;
        if let Some(info) = probe_java_until(&canon, deadline.min(Instant::now() + PROBE_TIMEOUT)) {
            seen_dirs.insert(dir);
            out.push(info);
        }
    }
    out
}

/// Find `bin/javaw.exe` (or `java`) under `<root>/java/**` (root is already
/// the `runtimes` directory).
fn runtime_java_paths(root: Option<&Path>, budget: &mut ScanBudget) -> Vec<PathBuf> {
    let Some(base) = root.map(|r| r.join("java")) else {
        return Vec::new();
    };
    find_runtime_bins(&base, budget)
}

fn find_runtime_bins(base: &Path, budget: &mut ScanBudget) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !base.is_dir() {
        return out;
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize, budget: &mut ScanBudget) {
        if depth > 4 {
            return;
        }
        let java = dir.join("bin").join(java_exe());
        if java.is_file() {
            out.push(java);
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            if !budget.visit() || out.len() >= MAX_CANDIDATES {
                return;
            }
            let p = e.path();
            if e.metadata().is_ok_and(|metadata| metadata.is_dir() && !crate::fsutil::is_link_or_reparse_point(&metadata)) {
                walk(&p, out, depth + 1, budget);
            }
        }
    }
    walk(base, &mut out, 0, budget);
    out
}

pub fn java_exe() -> &'static str {
    if cfg!(windows) {
        "javaw.exe"
    } else {
        "java"
    }
}

/// Run `java -version` and parse the first line, e.g.
/// `openjdk version "21.0.2" 2024-01-16` or `java version "1.8.0_392"`.
pub fn probe_java(path: &Path) -> Option<JavaInfo> {
    probe_java_until(path, Instant::now() + PROBE_TIMEOUT)
}

fn probe_java_until(path: &Path, deadline: Instant) -> Option<JavaInfo> {
    let mut cmd = Command::new(path);
    cmd.arg("-version");
    let output = probe_output(&mut cmd, deadline).ok()?;
    if !output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = if stderr.contains("version \"") { &stderr } else { &stdout };
    let first = text.lines().find(|line| line.contains("version \"")).unwrap_or("");

    let mut version = String::new();
    let mut major: u32 = 0;
    let mut vendor = String::new();

    if let Some(idx) = first.find("version \"") {
        let rest = &first[idx + 9..];
        let v = rest.split('"').next().unwrap_or("");
        version = v.to_string();
        if let Some(dot) = v.find('.') {
            // 1.8.0_392 -> 8 ; 21.0.2 -> 21
            let head = &v[..dot];
            if head == "1" {
                let after = &v[dot + 1..];
                if let Some(dot2) = after.find('.') {
                    major = after[..dot2].parse().unwrap_or(0);
                } else {
                    major = after.parse().unwrap_or(0);
                }
            } else {
                major = head.parse().unwrap_or(0);
            }
        } else {
            major = v.parse().unwrap_or(0);
        }
    }
    for line in text.lines().skip(1).take(2) {
        let l = line.trim();
        if l.contains("OpenJDK") || l.contains("Eclipse") || l.contains("Oracle") || l.contains("Microsoft") || l.contains("Temurin") || l.contains("Zulu") {
            vendor = l.to_string();
            break;
        }
    }
    if vendor.is_empty() {
        let p = path.to_string_lossy().to_lowercase();
        if p.contains("temurin") || p.contains("adoptium") {
            vendor = "Eclipse Temurin".into();
        } else if p.contains("zulu") || p.contains("azul") {
            vendor = "Azul Zulu".into();
        } else if p.contains("microsoft") || p.contains("openjdk") {
            vendor = "Microsoft OpenJDK".into();
        } else if p.contains("oracle") {
            vendor = "Oracle".into();
        } else {
            vendor = "Unknown".into();
        }
    }
    if major == 0 {
        return None;
    }
    Some(JavaInfo {
        path: clean_path(path),
        version,
        major,
        vendor,
        arch: probe_arch(path, deadline),
    })
}

/// Strip the Windows verbatim prefix (`\\?\`) from canonicalized paths.
fn clean_path(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s
    }
}

fn path_key(path: &Path) -> String {
    let name = path.to_string_lossy();
    if cfg!(windows) { name.to_lowercase() } else { name.into_owned() }
}

fn probe_arch(path: &Path, deadline: Instant) -> String {
    let mut cmd = Command::new(path);
    cmd.args(["-XshowSettings:properties", "-version"]);
    let output = probe_output(&mut cmd, deadline);
    if let Ok(o) = output {
        let text = String::from_utf8_lossy(&o.stderr).to_string();
        for line in text.lines() {
            let t = line.trim();
            if let Some(v) = t.strip_prefix("os.arch =") {
                return v.trim().to_string();
            }
        }
    }
    if cfg!(target_arch = "aarch64") {
        "aarch64".into()
    } else {
        "x86_64".into()
    }
}

/// Read Java install paths from the Windows registry.
///
/// Mirrors what PrismLauncher / PCL search: JavaSoft (old + new key names),
/// AdoptOpenJDK, Eclipse Foundation, Eclipse Adoptium, Semeru, Microsoft JDK,
/// Azul Zulu and BellSoft Liberica, in both HKLM/HKCU and 32/64-bit views.
#[cfg(windows)]
fn registry_java_paths(deadline: Instant) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let registry_deadline = deadline.min(Instant::now() + Duration::from_secs(4));
    let roots = [
        "SOFTWARE\\JavaSoft\\Java Runtime Environment",
        "SOFTWARE\\JavaSoft\\Java Development Kit",
        "SOFTWARE\\JavaSoft\\JRE",
        "SOFTWARE\\JavaSoft\\JDK",
        "SOFTWARE\\AdoptOpenJDK",
        "SOFTWARE\\Eclipse Foundation",
        "SOFTWARE\\Eclipse Adoptium",
        "SOFTWARE\\Semeru",
        "SOFTWARE\\Microsoft\\JDK",
        "SOFTWARE\\Microsoft\\JavaSoft",
        "SOFTWARE\\Azul Systems",
        "SOFTWARE\\BellSoft",
        "SOFTWARE\\Amazon Corretto",
    ];
    for hive in ["HKLM", "HKCU"] {
        for root in roots {
            let key = format!("{hive}\\{root}");
            for reg_view in ["", "/reg:32"] {
                if Instant::now() >= registry_deadline || out.len() >= MAX_CANDIDATES {
                    return out;
                }
                let mut args = vec!["query", &key, "/s"];
                if !reg_view.is_empty() {
                    args.push(reg_view);
                }
                let mut reg_cmd = Command::new("reg");
                reg_cmd.args(&args);
                let query_deadline = registry_deadline.min(Instant::now() + Duration::from_millis(350));
                if let Ok(o) = probe_output(&mut reg_cmd, query_deadline) {
                    let text = String::from_utf8_lossy(&o.stdout).to_string();
                    for line in text.lines() {
                        let t = line.trim();
                        // value lines look like:  JavaHome  REG_SZ  C:\...
                        let parts: Vec<&str> = t.split_whitespace().collect();
                        if parts.len() >= 3
                            && (parts[1] == "REG_SZ" || parts[1] == "REG_EXPAND_SZ")
                            && matches!(parts[0], "JavaHome" | "InstallPath" | "Path" | "Home")
                        {
                            let mut v = parts[2..].join(" ");
                            v = v.trim_matches('"').to_string();
                            if !v.is_empty() {
                                out.push(PathBuf::from(v).join("bin").join(java_exe()));
                                if out.len() >= MAX_CANDIDATES {
                                    return out;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn registry_java_paths(_deadline: Instant) -> Vec<PathBuf> {
    Vec::new()
}

/// Deep filesystem search for `javaw.exe` / `java.exe` in every place Java
/// installs can hide: Program Files (recursively), IDE-bundled JBRs, Oracle
/// javapath, user JDK dirs (~/.jdks, ~/.gradle/jdks, sdkman), the official
/// launcher's runtimes and launcher-managed runtimes.
fn fs_scan_paths(budget: &mut ScanBudget) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok();
    let appdata = std::env::var("APPDATA").ok();
    let local = std::env::var("LOCALAPPDATA").ok();
    let program_files = std::env::var("ProgramFiles").ok();
    let program_files_x86 = std::env::var("ProgramFiles(x86)").ok();

    let mut bases: Vec<PathBuf> = Vec::new();
    if let Some(p) = &program_files {
        bases.push(PathBuf::from(p));
    }
    if let Some(p) = &program_files_x86 {
        bases.push(PathBuf::from(p));
    }
    if let Some(p) = &local {
        bases.push(PathBuf::from(p).join("Programs"));
        bases.push(PathBuf::from(p).join("JetBrains")); // Toolbox JBRs
    }
    if let Some(p) = &program_files {
        // Oracle auto-update entry point
        bases.push(PathBuf::from(p).join("Common Files").join("Oracle").join("Java").join("javapath"));
    }
    if let Some(h) = &home {
        bases.push(PathBuf::from(h).join(".jdks"));
        bases.push(PathBuf::from(h).join(".gradle").join("jdks"));
        bases.push(PathBuf::from(h).join(".sdkman").join("candidates").join("java"));
        bases.push(PathBuf::from(h).join(".minecraft").join("runtime"));
    }
    if let Some(a) = &appdata {
        bases.push(PathBuf::from(a).join(".minecraft").join("runtime"));
    }

    // deny-list of huge unrelated dirs inside Program Files scans
    let deny = [
        "windows kits",
        "dotnet",
        "microsoft visual studio",
        "windowsapps",
        "common files",
        "reference assemblies",
        "nuget",
        "windows defender",
        "internet explorer",
        "windows nt",
        "windows multimedia platform",
        "microsoft office",
        "windows security",
        "windows portable devices",
        "windows sidebar",
    ];

    for base in bases {
        if !base.exists() {
            continue;
        }
        scan_java_exe(&base, &mut out, 0, &deny, budget);
    }
    out
}

fn scan_java_exe(dir: &Path, out: &mut Vec<PathBuf>, depth: usize, deny: &[&str], budget: &mut ScanBudget) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if !budget.visit() || out.len() >= MAX_CANDIDATES {
            return;
        }
        let p = e.path();
        let Ok(md) = e.metadata() else { continue };
        if md.is_dir() && !crate::fsutil::is_link_or_reparse_point(&md) {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if deny.contains(&name.as_str()) {
                continue;
            }
            scan_java_exe(&p, out, depth + 1, deny, budget);
        } else {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n == java_exe() || (cfg!(windows) && n == "java.exe") {
                out.push(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_java() -> Vec<JavaInfo> {
        vec![JavaInfo {
            path: "C:\\Java\\bin\\javaw.exe".into(),
            version: "17.0.2".into(),
            major: 17,
            vendor: "Adoptium".into(),
            arch: "x86_64".into(),
        }]
    }

    #[test]
    fn cache_round_trip_save_load_invalidate() {
        let dir = std::env::temp_dir().join(format!(
            "qk-java-cache-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let list = sample_java();
        let now = 1_700_000_000u64;
        save_java_cache(&dir, &list, now);
        assert_eq!(load_java_cache(&dir), Some((now, list)));

        invalidate_java_cache(&dir);
        assert!(load_java_cache(&dir).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_detect_returns_fresh_disk_cache_without_rescan() {
        let dir = std::env::temp_dir().join(format!(
            "qk-java-cache-test2-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        // A "fresh" cache (100s old, far below the 30-day TTL) must be
        // returned as-is — no scan should happen.
        let list = sample_java();
        save_java_cache(&dir, &list, 1_700_000_000);
        let now = 1_700_000_100;
        let (ts, out) = cached_detect(&dir, &dir, now, false);
        assert_eq!(out, list);
        assert_eq!(ts, now);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_detect_force_ignores_disk_cache() {
        let dir = tempfile::tempdir().unwrap();
        save_java_cache(dir.path(), &sample_java(), 1_700_000_000);
        let now = 1_700_000_100;
        let (_, out) = detect_with_cache(dir.path(), now, true, Vec::new);
        assert!(out.is_empty());
        assert_eq!(load_java_cache(dir.path()), Some((now, out)));
    }

    #[test]
    #[ignore = "run in a child by timed_probe_kills_a_hanging_candidate"]
    fn blocking_probe_fixture() {
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    fn timed_probe_kills_a_hanging_candidate() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--ignored", "--exact", "java::tests::blocking_probe_fixture"]);
        let start = Instant::now();
        let error = probe_output(&mut command, start + Duration::from_millis(250)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn nonzero_exit_is_not_accepted_as_java() {
        #[cfg(windows)]
        let mut command = { let mut cmd = Command::new("cmd.exe"); cmd.args(["/D", "/C", "exit 23"]); cmd };
        #[cfg(not(windows))]
        let mut command = { let mut cmd = Command::new("/bin/sh"); cmd.args(["-c", "exit 23"]); cmd };
        let output = probe_output(&mut command, Instant::now() + Duration::from_secs(10)).unwrap();
        assert_eq!(output.status.code(), Some(23));
    }
}
