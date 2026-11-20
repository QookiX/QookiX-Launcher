use crate::models::{ManifestVersion, VersionJson, VersionManifest};
use crate::state::AppState;
use std::path::PathBuf;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const BMCLAPI_MANIFEST: &str = "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json";

/// Fetch and cache the Mojang version manifest.
pub async fn fetch_manifest(state: &AppState) -> Result<VersionManifest, String> {
    let path = state.root.join("version_manifest.json");
    let primary = crate::mirror::manifest_url(state);
    let fallback = if primary == MANIFEST_URL {
        BMCLAPI_MANIFEST
    } else {
        MANIFEST_URL
    };
    let client = &state.client;
    load_manifest(&path, [&primary, fallback], |url| async move {
        crate::download::get_text(client, &url).await
    })
    .await
}

async fn load_manifest<F, R>(
    path: &std::path::Path,
    sources: [&str; 2],
    mut fetch: F,
) -> Result<VersionManifest, String>
where
    F: FnMut(String) -> R,
    R: std::future::Future<Output = Result<String, String>>,
{
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age.as_secs() <= 3600);
    let cached = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| parse_manifest(&text).ok());
    let cached = match (fresh, cached) {
        (true, Some(manifest)) => return Ok(manifest),
        (_, cached) => cached,
    };
    let mut errors = Vec::new();
    for (index, source) in sources.into_iter().enumerate() {
        match fetch(source.to_string())
            .await
            .and_then(|text| parse_manifest(&text).map(|manifest| (manifest, text)))
        {
            Ok((manifest, text)) => {
                crate::fsutil::fs_best_effort(
                    "write",
                    path,
                    crate::fsutil::atomic_write(path, text.as_bytes()),
                );
                return Ok(manifest);
            }
            Err(err) => errors.push(format!("来源 {}：{err}", index + 1)),
        }
    }
    cached.ok_or_else(|| format!("获取 Minecraft 版本清单失败（{}）", errors.join("；")))
}

fn parse_manifest(text: &str) -> Result<VersionManifest, String> {
    let manifest: VersionManifest = serde_json::from_str(text).map_err(|err| err.to_string())?;
    if [&manifest.latest.release, &manifest.latest.snapshot]
        .iter()
        .any(|id| id.is_empty() || find_version(&manifest, id).is_none())
    {
        return Err("版本清单缺少最新正式版或快照版".into());
    }
    Ok(manifest)
}

/// Find a manifest version entry by id.
pub fn find_version<'a>(manifest: &'a VersionManifest, id: &str) -> Option<&'a ManifestVersion> {
    manifest.versions.iter().find(|v| v.id == id)
}

/// Return the full (non-patched) Mojang version JSON for a version id,
/// following `inheritsFrom` chains.
pub async fn fetch_version_json(state: &AppState, id: &str) -> Result<VersionJson, String> {
    let manifest = fetch_manifest(state).await?;
    let entry = find_version(&manifest, id).ok_or_else(|| format!("未找到 Minecraft 版本 {id}"))?;
    let url = crate::mirror::version_json_url(state, &entry.url, id);
    // 官方直连失败（国内常态）时回退 BMCLAPI 镜像
    const BMCLAPI_BASE: &str = "https://bmclapi2.bangbang93.com";
    let json: VersionJson = if url == entry.url {
        match crate::download::get_json(&state.client, &url).await {
            Ok(j) => j,
            Err(e) => crate::download::get_json(
                &state.client,
                &format!("{BMCLAPI_BASE}/version/{id}/json"),
            )
            .await
            .map_err(|e2| format!("{e}（镜像源也失败: {e2}）"))?,
        }
    } else {
        match crate::download::get_json(&state.client, &url).await {
            Ok(j) => j,
            Err(_) => crate::download::get_json(&state.client, &entry.url).await?,
        }
    };
    Ok(json)
}

/// Cache a version JSON at `versions/<id>/<id>.json` (used for vanilla).
pub async fn cache_version_json(state: &AppState, json: &VersionJson) -> Result<PathBuf, String> {
    let dir = crate::paths::resolve_version_dir(state, &json.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", json.id));
    let text = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Resolve a version json following the `inheritsFrom` chain and merging
/// parent data (libraries, asset index, downloads, java version...).
#[allow(dead_code)]
pub fn resolve_inheritance(root: &VersionJson, parents: &[VersionJson]) -> VersionJson {
    let mut merged = root.clone();
    for parent in parents.iter().rev() {
        if merged.asset_index.is_none() {
            merged.asset_index = parent.asset_index.clone();
        }
        if merged.downloads.client.is_none() {
            merged.downloads.client = parent.downloads.client.clone();
        }
        if merged.downloads.server.is_none() {
            merged.downloads.server = parent.downloads.server.clone();
        }
        if merged.java_version.is_none() {
            merged.java_version = parent.java_version.clone();
        }
        if merged.logging.is_none() {
            merged.logging = parent.logging.clone();
        }
        // prepend parent libraries that are not already present (by name)
        let existing: std::collections::HashSet<String> =
            merged.libraries.iter().map(|l| l.name.clone()).collect();
        let mut libs: Vec<_> = parent
            .libraries
            .iter()
            .filter(|l| !existing.contains(&l.name))
            .cloned()
            .collect();
        libs.extend(merged.libraries.clone());
        merged.libraries = libs;
        if merged.minecraft_arguments.is_none() {
            merged.minecraft_arguments = parent.minecraft_arguments.clone();
        }
        if merged.arguments.is_none() {
            merged.arguments = parent.arguments.clone();
        }
    }
    merged.inherits_from = None;
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::ready;
    use std::time::{Duration, SystemTime};

    const MANIFEST: &str = r#"{
        "latest":{"release":"1.0","snapshot":"1.0"},
        "versions":[{"id":"1.0","type":"release","url":"https://example.invalid",
        "time":"2020-01-01T00:00:00Z","releaseTime":"2020-01-01T00:00:00Z"}]
    }"#;

    #[tokio::test]
    async fn invalid_response_tries_next_source_and_caches_only_the_valid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let mut hits = Vec::new();
        let manifest = load_manifest(&path, ["mirror", "official"], |source| {
            let response = if source == "mirror" {
                r#"{"latest":{"release":"1.0","snapshot":"1.0"},"versions":[]}"#
            } else {
                MANIFEST
            };
            hits.push(source);
            ready(Ok(response.into()))
        })
        .await
        .unwrap();
        assert_eq!(hits, ["mirror", "official"]);
        assert_eq!(manifest.latest.release, "1.0");
        assert_eq!(std::fs::read_to_string(path).unwrap(), MANIFEST);
    }

    #[tokio::test]
    async fn stale_valid_cache_survives_all_source_failures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        std::fs::write(&path, MANIFEST).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        let mut hits = 0;
        let manifest = load_manifest(&path, ["official", "mirror"], |_| {
            hits += 1;
            ready(Err("offline".into()))
        })
        .await
        .unwrap();
        assert_eq!(hits, 2);
        assert_eq!(manifest.latest.release, "1.0");
        assert_eq!(std::fs::read_to_string(path).unwrap(), MANIFEST);
    }
}
