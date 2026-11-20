//! 模组、整合包与资源包的元数据和图标。

use std::{io::Read, path::Path};

/// Extract the modpack icon from a zip archive and save it to
/// `instances/{instance_id}/pack-icon.png`. Returns the absolute path on success.
pub fn extract_modpack_icon(pack_path: &Path, instance_dir: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(pack_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for name in ["pack.png", "icon.png", "logo.png", "modpack.png"] {
        if let Ok(mut entry) = archive.by_name(name) {
            if entry.is_dir() {
                continue;
            }
            let mut buf = Vec::new();
            if Read::read_to_end(&mut entry, &mut buf).is_err() || buf.len() < 8 {
                continue;
            }
            let out = instance_dir.join("pack-icon.png");
            if std::fs::write(&out, &buf).is_ok() {
                return Some(out.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Download a remote image (e.g. modpack project icon) and save it to
/// `instances/{instance_id}/pack-icon.png`. Returns the absolute path on success.
pub async fn download_icon(
    client: &reqwest::Client,
    url: &str,
    instance_dir: &std::path::Path,
) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    let resp = client.get(url).send().await.ok()?;
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() < 8 {
        return None;
    }
    std::fs::create_dir_all(instance_dir).ok()?;
    let out = instance_dir.join("pack-icon.png");
    std::fs::write(&out, &bytes).ok()?;
    Some(out.to_string_lossy().to_string())
}

/// Best-effort: extract an icon embedded in a local mod / resource-pack / shader zip.
/// Looks for `pack.png`, `icon.png` or a Modrinth metadata icon; falls back to a generic icon name.
pub fn extract_archive_icon(path: &std::path::Path, kind: &str) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let image_candidates: &[&str] = match kind {
        "resourcepack" => &["pack.png", "icon.png", "assets/minecraft/textures/gui/title/icon.png"],
        "shader" => &["icon.png", "pack.png"],
        _ => &["icon.png", "pack.png"],
    };
    for name in image_candidates {
        if let Ok(mut entry) = archive.by_name(name) {
            if entry.is_dir() {
                continue;
            }
            let mut buf = Vec::new();
            Read::read_to_end(&mut entry, &mut buf).ok()?;
            if let Some(p) = save_icon_buf(buf) {
                return Some(p);
            }
        }
    }
    let mut icon_ref: Option<String> = None;
    for name in ["modrinth.mod.json", "fabric.mod.json"] {
        if icon_ref.is_some() {
            break;
        }
        if let Ok(mut entry) = archive.by_name(name) {
            let mut buf = Vec::new();
            Read::read_to_end(&mut entry, &mut buf).ok()?;
            let val: serde_json::Value = serde_json::from_slice(&buf).ok()?;
            if let Some(icon) = val.get("icon").and_then(|i| i.as_str()) {
                if !icon.is_empty() {
                    icon_ref = Some(icon.to_string());
                }
            }
        }
    }
    if let Some(icon) = icon_ref {
        if icon.starts_with("http://") || icon.starts_with("https://") {
            return Some(icon);
        }
        if let Ok(mut icon_entry) = archive.by_name(&icon) {
            let mut ibuf = Vec::new();
            Read::read_to_end(&mut icon_entry, &mut ibuf).ok()?;
            if let Some(p) = save_icon_buf(ibuf) {
                return Some(p);
            }
        }
    }
    None
}

/// jar 内声明的单个依赖，供依赖体检使用。
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ModDependency {
    /// 依赖方声明的 mod id（如 "fabric-api" / "forge"）
    pub mod_id: String,
    /// 版本范围原文，不做语义解析（"*" / ">=1.20" / "[1.20,1.21)"）
    pub requirement: String,
    /// required / optional
    pub kind: String,
}

/// 从 jar 内部解析出的 mod 元数据。
#[derive(Default, Clone, Debug)]
pub struct ModJarMeta {
    pub mod_id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub authors: Option<Vec<String>>,
    pub description: Option<String>,
    /// 已落地的 icon 本地路径或 http(s) URL
    pub icon: Option<String>,
    /// fabric / quilt / forge / neoforge
    pub loader: Option<String>,
    /// 声明的依赖（required + optional）
    pub depends: Vec<ModDependency>,
    /// 声明的 Minecraft 版本范围（原样，仅用于展示）
    pub mc_requirement: Option<String>,
}

fn save_icon_buf(buf: Vec<u8>) -> Option<String> {
    if buf.len() < 8 {
        return None;
    }
    // 内嵌为 data URL，不写临时文件——避免 temp 被清理后图标 404
    let mime = if buf.starts_with(b"\x89PNG") {
        "image/png"
    } else if buf.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if buf.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/png"
    };
    use base64::Engine as _;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&buf)
    ))
}

fn parse_json_authors(val: &serde_json::Value) -> Option<Vec<String>> {
    let a = val.get("authors")?;
    if let Some(arr) = a.as_array() {
        let v: Vec<String> = arr
            .iter()
            .filter_map(|x| {
                x.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| x.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
            })
            .filter(|s| !s.is_empty())
            .collect();
        if v.is_empty() { None } else { Some(v) }
    } else if let Some(s) = a.as_str() {
        if s.is_empty() { None } else { Some(vec![s.to_string()]) }
    } else {
        None
    }
}

/// 极简 mods.toml 值解析：支持 "str" / 'str' / ["a","b"] / bare。
fn parse_toml_value(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with('[') && s.ends_with(']') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        let items: Vec<String> = inner.split(',').filter_map(|item| parse_toml_value(item)).collect();
        return Some(items.join(", "));
    }
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        return Some(s[1..s.len() - 1].to_string());
    }
    if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2 {
        return Some(s[1..s.len() - 1].to_string());
    }
    Some(s.to_string())
}

fn split_authors(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// 解析 Forge/NeoForge 的 mods.toml：只取第一个 `[[mods]]` 块作为自身元数据，
/// 同时收集所有 `[[dependencies.<modid>]]` 块用于依赖体检。
fn parse_mods_toml(text: &str) -> Option<ModJarMeta> {
    let mut in_mods = false;
    let mut in_dep = false;
    let mut meta = ModJarMeta::default();
    let mut found_mod_id = false;
    let mut logo_file: Option<String> = None;
    // 当前依赖块的暂存字段
    let mut dep_id = String::new();
    let mut dep_range = String::new();
    let mut dep_required = true;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[[") {
            if in_dep {
                push_dep_block(&mut meta, &dep_id, &dep_range, dep_required);
                dep_id.clear();
                dep_range.clear();
                dep_required = true;
            }
            // 只认第一个 [[mods]] 块，后续同名块不再覆盖元数据
            in_mods = trimmed == "[[mods]]" && !found_mod_id;
            in_dep = trimmed.starts_with("[[dependencies.");
            continue;
        }
        if trimmed.starts_with('[') {
            if in_dep {
                push_dep_block(&mut meta, &dep_id, &dep_range, dep_required);
                dep_id.clear();
                dep_range.clear();
                dep_required = true;
                in_dep = false;
            }
            in_mods = false;
            continue;
        }
        let Some(eq) = trimmed.find('=') else { continue };
        let key = trimmed[..eq].trim();
        let value = trimmed[eq + 1..].trim();
        let Some(v) = parse_toml_value(value) else { continue };

        if in_dep {
            match key {
                "modId" => dep_id = v,
                "versionRange" => dep_range = v,
                // 旧格式（Forge）：mandatory = true / false
                "mandatory" => dep_required = !v.eq_ignore_ascii_case("false"),
                // 新格式（NeoForge）：type = "required" | "optional" | "incompatible"
                "type" => {
                    let t = v.to_ascii_lowercase();
                    if t == "incompatible" {
                        // 冲突声明不属于「缺失依赖」，先整体丢弃
                        dep_id.clear();
                        dep_range.clear();
                    } else {
                        dep_required = t != "optional";
                    }
                }
                _ => {}
            }
            continue;
        }
        if !in_mods {
            continue;
        }
        match key {
            "modId" => { meta.mod_id = Some(v); found_mod_id = true; }
            "displayName" => { meta.name = Some(v); }
            "version" => { meta.version = Some(v); }
            "authors" => { meta.authors = Some(split_authors(&v)); }
            "description" => { meta.description = Some(v); }
            "logoFile" | "logo" => { logo_file = Some(v); }
            _ => {}
        }
    }
    if in_dep {
        push_dep_block(&mut meta, &dep_id, &dep_range, dep_required);
    }
    if found_mod_id {
        meta.icon = logo_file;
        Some(meta)
    } else {
        None
    }
}

/// 把解析出的一个依赖块并入元数据；`minecraft` 额外记为 Minecraft 版本范围。
fn push_dep_block(meta: &mut ModJarMeta, mod_id: &str, range: &str, required: bool) {
    let id = mod_id.trim();
    if id.is_empty() {
        return;
    }
    let range = range.trim();
    if id.eq_ignore_ascii_case("minecraft") && meta.mc_requirement.is_none() && !range.is_empty() {
        meta.mc_requirement = Some(range.to_string());
    }
    meta.depends.push(ModDependency {
        mod_id: id.to_string(),
        requirement: range.to_string(),
        kind: if required { "required" } else { "optional" }.into(),
    });
}

/// 解析 fabric.mod.json / quilt.mod.json 的依赖声明。
///
/// - fabric：`depends` / `recommends` 是 `{ "mod-id": "版本范围" }` 映射
/// - quilt：字段位于 `quilt_loader` 下，且 `depends` 形如 `[{ "id": .., "versions": .. }]`
fn parse_json_dependencies(
    val: &serde_json::Value,
    quilt: bool,
) -> (Vec<ModDependency>, Option<String>) {
    let root = if quilt { val.get("quilt_loader").unwrap_or(val) } else { val };
    let mut deps: Vec<ModDependency> = Vec::new();
    let mut mc_requirement: Option<String> = None;

    for (field, kind) in [("depends", "required"), ("recommends", "optional")] {
        let Some(node) = root.get(field) else { continue };
        // 对象形式（fabric）
        if let Some(map) = node.as_object() {
            for (id, v) in map {
                let requirement = json_requirement_text(v);
                note_mc(&id, &requirement, &mut mc_requirement);
                deps.push(ModDependency {
                    mod_id: id.clone(),
                    requirement,
                    kind: kind.into(),
                });
            }
            continue;
        }
        // 数组形式（quilt）
        if let Some(arr) = node.as_array() {
            for item in arr {
                let Some(id) = item.get("id").and_then(|v| v.as_str()) else { continue };
                let requirement = item
                    .get("versions")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                note_mc(id, &requirement, &mut mc_requirement);
                deps.push(ModDependency {
                    mod_id: id.to_string(),
                    requirement,
                    kind: kind.into(),
                });
            }
        }
    }
    (deps, mc_requirement)
}

fn note_mc(id: &str, requirement: &str, slot: &mut Option<String>) {
    if slot.is_none() && !requirement.is_empty() && id.eq_ignore_ascii_case("minecraft") {
        *slot = Some(requirement.to_string());
    }
}

/// fabric 的版本范围既可能是字符串，也可能是字符串数组。
fn json_requirement_text(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", ");
    }
    String::new()
}

/// 从 jar 内部解析 mod 元数据（fabric/quilt/forge/neoforge 通用）。
/// 优先级：fabric.mod.json → quilt.mod.json → META-INF/mods.toml → META-INF/neoforge.mods.toml
/// 一次 zip 读取同时提取元数据与内嵌 icon。
pub fn parse_mod_jar(path: &Path) -> Option<ModJarMeta> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut meta = ModJarMeta::default();
    let mut icon_ref: Option<String> = None;
    let mut found = false;

    for name in ["fabric.mod.json", "quilt.mod.json"] {
        if found { break; }
        let Ok(mut entry) = archive.by_name(name) else { continue };
        let mut buf = Vec::new();
        if Read::read_to_end(&mut entry, &mut buf).is_err() { continue; }
        let val: serde_json::Value = match serde_json::from_slice(&buf) { Ok(v) => v, Err(_) => continue };
        meta.mod_id = val.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        meta.name = val.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
        meta.version = val.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
        meta.description = val.get("description").and_then(|v| v.as_str()).map(|s| s.to_string());
        meta.authors = parse_json_authors(&val);
        icon_ref = val.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());
        meta.loader = if name == "fabric.mod.json" { Some("fabric".into()) } else { Some("quilt".into()) };
        let (deps, mc_req) = parse_json_dependencies(&val, name == "quilt.mod.json");
        meta.depends = deps;
        meta.mc_requirement = mc_req;
        found = true;
    }

    for name in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if found { break; }
        let Ok(mut entry) = archive.by_name(name) else { continue };
        let mut buf = Vec::new();
        if Read::read_to_end(&mut entry, &mut buf).is_err() { continue; }
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(m) = parse_mods_toml(&text) {
            icon_ref = m.icon.clone();
            meta = m;
            meta.loader = if name.contains("neoforge") { Some("neoforge".into()) } else { Some("forge".into()) };
            found = true;
        }
    }

    if !found { return None; }

    if let Some(icon) = icon_ref {
        if icon.starts_with("http://") || icon.starts_with("https://") {
            meta.icon = Some(icon);
        } else if let Ok(mut icon_entry) = archive.by_name(&icon) {
            let mut ibuf = Vec::new();
            if Read::read_to_end(&mut icon_entry, &mut ibuf).is_ok() {
                meta.icon = save_icon_buf(ibuf);
            }
        }
    }
    Some(meta)
}

/// 从 zip 内 `pack.mcmeta` 解析材质包/光影包的描述与 pack_format。
fn parse_pack_mcmeta(path: &Path) -> Option<(String, Option<i64>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let Ok(mut entry) = archive.by_name("pack.mcmeta") else { return None };
    let mut buf = Vec::new();
    if Read::read_to_end(&mut entry, &mut buf).is_err() { return None }
    let val: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    let pack = val.get("pack")?;
    let desc_str = pack.get("description").and_then(|d| json_text(d));
    let pack_format = pack.get("pack_format").and_then(|v| v.as_i64());
    desc_str.map(|d| (d, pack_format))
}

/// 从 zip 根目录提取 `pack.png` 作为图标，保存到临时文件并返回路径。
fn extract_zip_icon(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let Ok(mut entry) = archive.by_name("pack.png") else { return None };
    let mut buf = Vec::new();
    if Read::read_to_end(&mut entry, &mut buf).is_err() { return None }
    save_icon_buf(buf)
}

/// 从 JSON 文本组件提取纯文本（支持 string / object / array 三种形式）。
fn json_text(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(o) => {
            o.get("text").and_then(|v| v.as_str())
                .or_else(|| o.get("fallback").and_then(|v| v.as_str()))
                .map(|s| s.to_string())
        }
        serde_json::Value::Array(arr) => {
            let mut out = String::new();
            for item in arr {
                if let Some(t) = json_text(item) { out.push_str(&t); }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}

/// 用 jar/zip 内部元数据回填 `InstalledContent` 中缺失的字段。
/// 先尝试 mod jar 元数据（fabric/forge…），再回退到 pack.mcmeta（材质包/光影包）。
pub fn fill_content_from_jar(rec: &mut crate::models::InstalledContent, jar_path: &Path) {
    if let Some(meta) = parse_mod_jar(jar_path) {
        if rec.mod_id.is_none() { rec.mod_id = meta.mod_id; }
        let name_is_placeholder = rec.name.as_deref() == Some(rec.filename.as_str());
        if rec.name.is_none() || name_is_placeholder {
            if let Some(n) = meta.name.filter(|n| !n.is_empty()) { rec.name = Some(n); }
        }
        if rec.version.is_none() { rec.version = meta.version; }
        if rec.authors.is_none() { rec.authors = meta.authors; }
        if rec.description.is_none() { rec.description = meta.description; }
        if rec.icon.is_none() { rec.icon = meta.icon; }
        if rec.slug.is_none() { rec.slug = rec.mod_id.clone(); }
        return;
    }
    if let Some((desc, _)) = parse_pack_mcmeta(jar_path) {
        let name_is_placeholder = rec.name.as_deref() == Some(rec.filename.as_str());
        if rec.name.is_none() || name_is_placeholder {
            let clean = strip_mc_formatting(&desc);
            if !clean.is_empty() { rec.name = Some(clean); }
        }
        if rec.description.is_none() { rec.description = Some(desc); }
    }
    if rec.icon.is_none() {
        if let Some(icon) = extract_zip_icon(jar_path) {
            rec.icon = Some(icon);
        }
    }
    let name_is_placeholder = rec.name.as_deref() == Some(rec.filename.as_str());
    if rec.name.is_none() || name_is_placeholder {
        let stem = rec.filename.trim_end_matches(".zip").trim_end_matches(".jar");
        let pretty: String = stem.replace('_', " ");
        if !pretty.is_empty() { rec.name = Some(pretty); }
    }
}

/// 去除 Minecraft §x 格式码（§ 及紧跟的一个字符）。
fn strip_mc_formatting(s: &str) -> String {
    let mut out = String::new();
    let mut skip_next = false;
    for c in s.chars() {
        if skip_next { skip_next = false; continue; }
        if c == '\u{00a7}' { skip_next = true; continue; }
        out.push(c);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fabric_dependencies() {
        let val: serde_json::Value = serde_json::from_str(
            r#"{
            "id": "sodium-extra",
            "depends": { "fabricloader": ">=0.14", "minecraft": "~1.20.1", "fabric-api": "*" },
            "recommends": { "modmenu": "*" }
        }"#,
        )
        .unwrap();
        let (deps, mc) = parse_json_dependencies(&val, false);
        assert_eq!(mc.as_deref(), Some("~1.20.1"));
        let find = |id: &str| deps.iter().find(|d| d.mod_id == id).unwrap();
        assert_eq!(find("fabric-api").kind, "required");
        assert_eq!(find("fabric-api").requirement, "*");
        assert_eq!(find("modmenu").kind, "optional");
        assert_eq!(find("fabricloader").requirement, ">=0.14");
    }

    #[test]
    fn parses_quilt_dependencies() {
        let val: serde_json::Value = serde_json::from_str(
            r#"{
            "quilt_loader": {
                "id": "my-mod",
                "depends": [
                    { "id": "quilt_loader", "versions": ">=0.19" },
                    { "id": "minecraft", "versions": "1.20.x" },
                    { "id": "quilted_fabric_api", "versions": ">=6" }
                ]
            }
        }"#,
        )
        .unwrap();
        let (deps, mc) = parse_json_dependencies(&val, true);
        assert_eq!(mc.as_deref(), Some("1.20.x"));
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[2].mod_id, "quilted_fabric_api");
        assert!(deps.iter().all(|d| d.kind == "required"));
    }

    #[test]
    fn fabric_version_range_may_be_array() {
        let val: serde_json::Value = serde_json::from_str(
            r#"{ "depends": { "fabric-api": [">=0.90", "<0.100"] } }"#,
        )
        .unwrap();
        let (deps, _) = parse_json_dependencies(&val, false);
        assert_eq!(deps[0].requirement, ">=0.90, <0.100");
    }

    #[test]
    fn parses_forge_toml_dependencies() {
        let text = r#"
modLoader="javafml"
loaderVersion="[47,)"
license="MIT"

[[mods]]
modId="examplemod"
version="1.0.0"
displayName="Example Mod"
description="demo"

[[dependencies.examplemod]]
    modId="forge"
    mandatory=true
    versionRange="[47,)"
    ordering="NONE"
    side="BOTH"

[[dependencies.examplemod]]
    modId="minecraft"
    mandatory=true
    versionRange="[1.20,1.21)"

[[dependencies.examplemod]]
    modId="jei"
    mandatory=false
    versionRange="[15,)"
"#;
        let meta = parse_mods_toml(text).unwrap();
        assert_eq!(meta.mod_id.as_deref(), Some("examplemod"));
        assert_eq!(meta.name.as_deref(), Some("Example Mod"));
        assert_eq!(meta.mc_requirement.as_deref(), Some("[1.20,1.21)"));
        assert_eq!(meta.depends.len(), 3);
        assert_eq!(meta.depends[0].mod_id, "forge");
        assert_eq!(meta.depends[0].kind, "required");
        assert_eq!(meta.depends[2].mod_id, "jei");
        assert_eq!(meta.depends[2].kind, "optional");
    }

    #[test]
    fn parses_neoforge_toml_dependency_type() {
        let text = r#"
[[mods]]
modId="neoexample"
version="2.0"

[[dependencies.neoexample]]
modId="neoforge"
type="required"
versionRange="[21,)"

[[dependencies.neoexample]]
modId="some_lib"
type="optional"
versionRange="*"

[[dependencies.neoexample]]
modId="broken_mod"
type="incompatible"
versionRange="*"
"#;
        let meta = parse_mods_toml(text).unwrap();
        // incompatible 声明不参与缺失依赖
        assert_eq!(meta.depends.len(), 2);
        assert_eq!(meta.depends[0].mod_id, "neoforge");
        assert_eq!(meta.depends[0].kind, "required");
        assert_eq!(meta.depends[1].kind, "optional");
    }

    #[test]
    fn toml_without_mods_block_is_rejected() {
        let text = "[[dependencies.x]]\nmodId=\"forge\"\n";
        assert!(parse_mods_toml(text).is_none());
    }

    #[test]
    fn empty_requirement_keeps_minecraft_slot_unset() {
        let val: serde_json::Value =
            serde_json::from_str(r#"{ "depends": { "fabric-api": "*" } }"#).unwrap();
        let (deps, mc) = parse_json_dependencies(&val, false);
        assert_eq!(deps.len(), 1);
        assert!(mc.is_none());
    }

    // —— 新增函数的测试 ——
}
