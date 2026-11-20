//! Read the on-disk inventory separately from metadata reconciliation.
use crate::instances;
use crate::models::InstalledContent;
use crate::state::AppState;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentInventory {
    items: Vec<ContentRow>,
    on_disk: Vec<String>,
    warnings: Vec<String>,
    needs_reconciliation: bool,
}

#[derive(Serialize)]
struct ContentRow {
    record: ContentRecord,
    exists: bool,
}

#[derive(Serialize)]
struct ContentRecord {
    #[serde(flatten)]
    record: InstalledContent,
    cn_name: Option<String>,
}

struct DiskFile {
    path: PathBuf,
    size: u64,
    modified: u64,
}

#[derive(Default)]
struct DiskInventory {
    files: BTreeMap<String, DiskFile>,
    warnings: Vec<String>,
}

impl DiskInventory {
    fn read(folder: &Path) -> Result<Self, String> {
        let mut snapshot = Self::default();
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(snapshot),
            Err(error) => return Err(format!("读取内容目录失败: {error}")),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    snapshot.warnings.push(error.to_string());
                    continue;
                }
            };
            let Some(name) = entry.file_name().into_string().ok() else {
                snapshot.warnings.push("跳过无法显示的文件名".into());
                continue;
            };
            match fs::metadata(entry.path()) {
                Ok(metadata) if metadata.is_file() => {
                    snapshot.files.insert(name, DiskFile {
                        path: entry.path(),
                        size: metadata.len(),
                        modified: crate::fsutil::modified_secs(&metadata),
                    });
                }
                Ok(_) => {}
                Err(error) => snapshot.warnings.push(format!("{name}: {error}")),
            }
        }
        Ok(snapshot)
    }

    fn found(&self, filename: &str) -> Option<(&DiskFile, bool)> {
        if let Some(file) = self.files.get(filename) {
            Some((file, true))
        } else {
            self.files.get(&format!("{filename}.disabled")).map(|file| (file, false))
        }
    }

    fn add_unregistered(&self, records: &mut BTreeMap<String, InstalledContent>, kind: &str) {
        let extension = if kind == "mod" { "jar" } else { "zip" };
        for raw_name in self.files.keys() {
            let filename = raw_name.strip_suffix(".disabled").unwrap_or(raw_name);
            if !crate::fsutil::is_safe_filename(filename)
                || !Path::new(filename).extension().is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                continue;
            }
            records.entry(filename.into()).or_insert_with(|| {
                let (file, enabled) = self.found(filename).expect("filename was collected from files");
                InstalledContent {
                    filename: filename.into(),
                    source: "manual".into(),
                    project_id: None,
                    slug: None,
                    version_id: None,
                    name: None,
                    version: None,
                    mod_id: None,
                    authors: None,
                    description: None,
                    size: file.size,
                    installed_at: file.modified,
                    icon: None,
                    enabled,
                }
            });
        }
    }
}

fn row(mut record: InstalledContent, disk: &DiskInventory) -> ContentRow {
    let found = disk.found(&record.filename);
    if let Some((_, enabled)) = found {
        record.enabled = enabled;
    }
    let cn_name = crate::mcmod::cn_name_for_record(&record.source, record.slug.as_deref(), record.name.as_deref());
    ContentRow { exists: found.is_some(), record: ContentRecord { record, cn_name } }
}

fn needs_metadata(record: &InstalledContent) -> bool {
    record.name.as_deref().is_none_or(|name| name == record.filename)
}

pub fn list(folder: &Path, kind: &str, registered: Vec<InstalledContent>) -> Result<ContentInventory, String> {
    let disk = DiskInventory::read(folder)?;
    let mut discovered = BTreeMap::new();
    disk.add_unregistered(&mut discovered, kind);
    for record in &registered {
        discovered.remove(&record.filename);
    }
    let needs_reconciliation = !discovered.is_empty() || registered.iter().any(|record|
        needs_metadata(record) && disk.found(&record.filename).is_some());
    let items = registered.into_iter().chain(discovered.into_values()).map(|record| row(record, &disk)).collect();
    Ok(ContentInventory { items, on_disk: disk.files.into_keys().collect(), warnings: disk.warnings, needs_reconciliation })
}

pub struct Reconciliation {
    drafts: Vec<(DiskFile, Option<InstalledContent>, InstalledContent)>,
    warnings: Vec<String>,
}

pub fn inspect_archives(folder: &Path, kind: &str, registered: Vec<InstalledContent>) -> Result<Reconciliation, String> {
    let disk = DiskInventory::read(folder)?;
    let before: HashMap<_, _> = registered.into_iter().map(|record| (record.filename.clone(), record)).collect();
    let mut records = BTreeMap::new();
    disk.add_unregistered(&mut records, kind);
    for (name, record) in &before {
        records.insert(name.clone(), record.clone());
    }
    let mut drafts = Vec::new();
    for (filename, mut record) in records {
        let Some((file, _)) = disk.found(&filename) else { continue };
        let old = before.get(&filename);
        if old.is_some() && !needs_metadata(&record) {
            continue;
        }
        crate::content_metadata::fill_content_from_jar(&mut record, &file.path);
        drafts.push((
            DiskFile { path: file.path.clone(), size: file.size, modified: file.modified },
            old.cloned(),
            record,
        ));
    }
    Ok(Reconciliation { drafts, warnings: disk.warnings })
}

pub fn commit(state: &AppState, id: &str, kind: &str, pending: Reconciliation) -> Result<Vec<String>, String> {
    if pending.drafts.is_empty() {
        return Ok(pending.warnings);
    }
    instances::edit_instance(state, id, |instance| {
        let records = instances::content_records_mut(instance, kind);
        let mut changed = false;
        for (file, before, enriched) in pending.drafts {
            let still_present = fs::metadata(&file.path).is_ok_and(|meta|
                meta.is_file() && meta.len() == file.size && crate::fsutil::modified_secs(&meta) == file.modified);
            if !still_present {
                continue;
            }
            match (records.iter_mut().find(|record| record.filename == enriched.filename), before) {
                (None, None) => {
                    records.push(enriched);
                    changed = true;
                }
                (Some(current), Some(before)) if *current == before && *current != enriched => {
                    *current = enriched;
                    changed = true;
                }
                _ => {}
            }
        }
        Ok(changed)
    })?;
    Ok(pending.warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_reports_unregistered_files_without_hydrating_metadata() {
        let folder = tempfile::tempdir().unwrap();
        fs::write(folder.path().join("local_mod.jar.disabled"), "example").unwrap();
        fs::write(folder.path().join("notes.txt"), "ignore").unwrap();
        let listing = list(folder.path(), "mod", Vec::new()).unwrap();
        assert!(listing.needs_reconciliation);
        assert_eq!(listing.items.len(), 1);
        assert!(!listing.items[0].record.record.enabled);
        assert_eq!(listing.items[0].record.record.filename, "local_mod.jar");
        assert_eq!(listing.items[0].record.record.name, None);
        assert!(listing.items[0].exists);

        let plan = inspect_archives(folder.path(), "mod", Vec::new()).unwrap();
        assert_eq!(plan.drafts.len(), 1);
        assert_eq!(plan.drafts[0].2.name.as_deref(), Some("local mod"));
    }

    #[test]
    fn shows_missing_directory_as_empty_and_non_directory_as_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let bad = root.path().join("file.txt");
        fs::write(&bad, "").unwrap();
        assert!(list(&missing, "mod", vec![]).unwrap().items.is_empty());
        assert!(list(&bad, "mod", vec![]).is_err());
    }
}
