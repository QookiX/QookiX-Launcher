use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "service", rename_all = "snake_case")]
pub(super) enum Service {
    Builtin,
    Custom { endpoint: String, model: String },
}

impl Service {
    pub(super) fn custom(endpoint: &str, model: &str) -> Self {
        let digest = |value: &str| format!("{:x}", Sha256::digest(value.as_bytes()));
        Self::Custom { endpoint: digest(endpoint), model: digest(model) }
    }

    fn group(&self) -> Group {
        match self {
            Self::Builtin => Group::Builtin,
            Self::Custom { .. } => Group::Custom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Purpose {
    Summary,
    Body,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Key {
    #[serde(flatten)]
    service: Service,
    platform: String,
    purpose: Purpose,
    language: String,
    id: String,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    text: String,
    ts: u64,
}

#[derive(Serialize, Deserialize)]
struct Record {
    key: Key,
    entry: Entry,
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    schema: u8,
    records: Vec<Record>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Current(DiskCache),
    Legacy(BTreeMap<String, Entry>),
}

#[derive(Clone, Copy, Ord, PartialOrd, Eq, PartialEq)]
enum Group {
    Builtin,
    Custom,
}

type Entries = BTreeMap<Key, Entry>;
type Epochs = BTreeMap<(PathBuf, Group), u64>;
static WRITE_GATE: LazyLock<Mutex<Epochs>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn cache_file(root: &Path) -> PathBuf {
    root.join("cache").join("translations.json")
}

fn legacy_key(key: &str) -> Option<Key> {
    let parts: Vec<_> = key.split(':').collect();
    let (platform, id, language, purpose) = match parts.as_slice() {
        [platform, id, language] => (*platform, *id, *language, Purpose::Summary),
        [platform, id, language, "body", "v2", service] if platform == service => {
            (*platform, *id, *language, Purpose::Body)
        }
        _ => return None,
    };
    if !matches!(platform, "modrinth" | "curseforge" | "spigot") || id.is_empty() {
        return None;
    }
    Some(Key { service: Service::Builtin, platform: platform.into(), purpose, language: language.into(), id: id.into() })
}

fn load(root: &Path) -> Result<(Entries, bool), String> {
    let text = match std::fs::read_to_string(cache_file(root)) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((Entries::new(), false)),
        Err(err) => return Err(format!("无法读取翻译缓存: {err}")),
    };
    match serde_json::from_str::<Stored>(&text).map_err(|err| format!("翻译缓存格式无效: {err}"))? {
        Stored::Current(cache) if cache.schema == 2 => {
            Ok((cache.records.into_iter().map(|record| (record.key, record.entry)).collect(), false))
        }
        Stored::Current(_) => Err("翻译缓存版本不受支持".into()),
        Stored::Legacy(entries) => Ok((
            entries.into_iter().filter_map(|(key, entry)| Some((legacy_key(&key)?, entry))).collect(),
            true,
        )),
    }
}

fn save(root: &Path, entries: Entries) -> Result<u64, String> {
    let path = cache_file(root);
    std::fs::create_dir_all(path.parent().ok_or("无效翻译缓存目录")?).map_err(|err| err.to_string())?;
    let records = entries.into_iter().map(|(key, entry)| Record { key, entry }).collect();
    let bytes = serde_json::to_vec(&DiskCache { schema: 2, records }).map_err(|err| err.to_string())?;
    crate::fsutil::atomic_write(&path, &bytes).map_err(|err| format!("写入翻译缓存失败: {err}"))?;
    Ok(bytes.len() as u64)
}

pub(super) struct CacheBatch<'a> {
    root: &'a Path,
    service: Service,
    platform: &'a str,
    purpose: Purpose,
    epoch: u64,
    now: u64,
    input: Entries,
    updates: Entries,
}

impl<'a> CacheBatch<'a> {
    pub(super) fn new(root: &'a Path, service: Service, platform: &'a str, purpose: Purpose, now: u64) -> Self {
        let gate = WRITE_GATE.lock().unwrap_or_else(|err| err.into_inner());
        let epoch = *gate.get(&(root.to_path_buf(), service.group())).unwrap_or(&0);
        let (input, _) = load(root).unwrap_or_default();
        Self { root, service, platform, purpose, epoch, now, input, updates: Entries::new() }
    }

    fn key(&self, id: &str) -> Key {
        Key {
            service: self.service.clone(), platform: self.platform.to_owned(), purpose: self.purpose,
            language: super::LANG.into(), id: id.into(),
        }
    }

    pub(super) fn get(&self, id: &str, ttl: u64) -> Option<String> {
        let key = self.key(id);
        let entry = self.updates.get(&key).or_else(|| self.input.get(&key))?;
        (self.now.saturating_sub(entry.ts) < ttl && !entry.text.is_empty()).then(|| entry.text.clone())
    }

    pub(super) fn add(&mut self, id: &str, text: &str) {
        self.updates.insert(self.key(id), Entry { text: text.to_owned(), ts: self.now });
    }

    pub(super) fn commit(self) {
        if self.updates.is_empty() { return; }
        if let Err(error) = self.save_all() {
            crate::util::log_line(&format!("[translate] cache commit failed: {error}"));
        }
    }

    fn save_all(self) -> Result<(), String> {
        let gate = WRITE_GATE.lock().unwrap_or_else(|err| err.into_inner());
        if gate.get(&(self.root.to_path_buf(), self.service.group())).copied().unwrap_or(0) != self.epoch {
            return Ok(());
        }
        let (mut latest, _) = load(self.root)?;
        for (key, entry) in self.updates {
            if latest.get(&key).is_none_or(|existing| existing.ts <= entry.ts) {
                latest.insert(key, entry);
            }
        }
        save(self.root, latest)?;
        Ok(())
    }
}

fn advance(epochs: &mut Epochs, root: &Path, group: Group) {
    let epoch = epochs.entry((root.to_path_buf(), group)).or_default();
    *epoch = epoch.wrapping_add(1);
}

pub(super) fn invalidate_builtin(root: &Path, platform: &str, id: &str) -> Result<(), String> {
    let mut gate = WRITE_GATE.lock().unwrap_or_else(|err| err.into_inner());
    let (mut entries, legacy) = load(root)?;
    let before = entries.len();
    entries.retain(|key, _| key.service != Service::Builtin || key.platform != platform || key.id != id);
    if legacy || before != entries.len() { save(root, entries)?; }
    advance(&mut gate, root, Group::Builtin);
    Ok(())
}

pub(super) fn clear(root: &Path, service: Option<&str>) -> Result<u64, String> {
    let group = match service {
        Some("default" | "modrinth") => Some(Group::Builtin),
        Some("custom") => Some(Group::Custom),
        None => None,
        _ => return Err("未知翻译服务".into()),
    };
    let mut gate = WRITE_GATE.lock().unwrap_or_else(|err| err.into_inner());
    let path = cache_file(root);
    let size = match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && !crate::fsutil::is_link_or_reparse_point(&meta) => meta.len(),
        Ok(_) => return Err("翻译缓存不是普通文件".into()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => 0,
        Err(err) => return Err(err.to_string()),
    };
    let freed = if let Some(group) = group {
        let (mut entries, legacy) = load(root)?;
        let before = entries.len();
        entries.retain(|key, _| key.service.group() != group);
        let freed = if before == entries.len() && !legacy {
            0
        } else {
            size.saturating_sub(save(root, entries)?)
        };
        advance(&mut gate, root, group);
        freed
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("清理翻译缓存失败: {err}")),
        }
        advance(&mut gate, root, Group::Builtin);
        advance(&mut gate, root, Group::Custom);
        size
    };
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_cache_batches_merge_and_model_and_purpose_do_not_mix() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let service = Service::custom("https://example.invalid/endpoint", "model-a");
        let mut summary = CacheBatch::new(root, service.clone(), "modrinth", Purpose::Summary, 1000);
        let mut body = CacheBatch::new(root, service.clone(), "curseforge", Purpose::Body, 1001);
        summary.add("demo", "summary");
        body.add("demo", "body");
        body.save_all().unwrap();
        summary.save_all().unwrap();
        let summary = CacheBatch::new(root, service.clone(), "modrinth", Purpose::Summary, 1002);
        let body = CacheBatch::new(root, service, "curseforge", Purpose::Body, 1002);
        assert_eq!(summary.get("demo", 100).as_deref(), Some("summary"));
        assert_eq!(body.get("demo", 100).as_deref(), Some("body"));
        assert!(body.get("demo", 1).is_none());
        let other_model = CacheBatch::new(root, Service::custom("https://example.invalid/endpoint", "model-b"), "modrinth", Purpose::Summary, 1002);
        assert!(other_model.get("demo", 100).is_none());
        let json = std::fs::read_to_string(cache_file(root)).unwrap();
        assert!(!json.contains("https://example.invalid"));
    }

    #[test]
    fn service_clear_covers_every_platform_and_prevents_inflight_reinsertions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut builtin = CacheBatch::new(root, Service::Builtin, "curseforge", Purpose::Body, 1000);
        builtin.add("old", "builtin");
        builtin.save_all().unwrap();
        let service = Service::custom("test", "model");
        let mut custom = CacheBatch::new(root, service, "modrinth", Purpose::Summary, 1001);
        custom.add("kept", "custom");
        custom.save_all().unwrap();
        let mut pending = CacheBatch::new(root, Service::Builtin, "modrinth", Purpose::Summary, 1002);
        pending.add("late", "not written");
        assert!(clear(root, Some("default")).unwrap() > 0);
        pending.save_all().unwrap();
        let (left, _) = load(root).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left.first_key_value().unwrap().0.id, "kept");
    }

    #[test]
    fn legacy_builtin_is_readable_but_unscoped_custom_cache_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let file = cache_file(root);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{
            "curseforge:example:zh":{"text":"builtin","ts":10},
            "custom:example:zh":{"text":"unscoped","ts":10}
        }"#).unwrap();
        let builtin = CacheBatch::new(root, Service::Builtin, "curseforge", Purpose::Summary, 12);
        assert_eq!(builtin.get("example", 10).as_deref(), Some("builtin"));
        clear(root, Some("custom")).unwrap();
        assert!(!std::fs::read_to_string(file).unwrap().contains("unscoped"));
    }
}
