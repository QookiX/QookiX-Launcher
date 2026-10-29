use crate::download::DownloadItem;
use crate::instances::add_content_batch;
use crate::models::{Instance, InstalledContent, LoaderType};
use crate::state::AppState;
use serde_json::Value;
use tauri::Emitter;

// ---------------------------------------------------------------------------
// 整合包安装公共骨架
//
// 三个来源（CurseForge 整合包 / Modrinth 整合包 / 分享包导入）的差异只在
// 「整包从哪来」与「文件清单怎么解析」，其余步骤完全一致：
//   建实例 → 图标 → 下载内容 → 登记内容记录 → 解压 overrides →
//   安装游戏本体 → 标记已安装
// 这里把公共部分收成一个函数，避免同一套流程维护三份。
// ---------------------------------------------------------------------------

/// 扫描实例 `mods/` 目录生成内容记录并登记（Modrinth 整合包安装后走这条路）。
/// `hash_meta` 提供「文件名 → (project_id, version_id)」，用于补上在线来源信息。
/// 返回登记条数。
pub fn scan_mod_records(
    state: &AppState,
    instance_id: &str,
    hash_meta: &std::collections::HashMap<String, (String, String)>,
) -> Result<usize, String> {
    let mods_dir = state.instances_dir().join(instance_id).join("mods");
    let mut records: Vec<InstalledContent> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&mods_dir) {
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jar") {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let meta = hash_meta.get(&name);
            let mut rec = InstalledContent {
                filename: name.clone(),
                source: "modrinth".into(),
                project_id: meta.map(|m| m.0.clone()),
                slug: None,
                version_id: meta.map(|m| m.1.clone()),
                name: Some(name),
                version: None,
                mod_id: None,
                authors: None,
                description: None,
                installed_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                size,
                icon: None,
                enabled: true,
            };
            crate::util::fill_content_from_jar(&mut rec, &path);
            records.push(rec);
        }
    }
    if records.is_empty() {
        return Ok(0);
    }
    let count = records.len();
    add_content_batch(state, instance_id, "mod", records)?;
    Ok(count)
}

/// 第一步：解析包元信息、建实例、处理图标。返回 (实例, 进度来源文案)。
/// 调用方拿到实例后再解析自己的文件清单（目标路径需要实例目录）。
pub struct PackPrepareSpec {
    /// 整包 zip 的本地路径（调用方需已下载完成）
    pub pack_path: std::path::PathBuf,
    /// zip 内无图标时的兜底图标 URL（项目 API 的 icon_url）
    pub icon_fallback_url: Option<String>,
}

pub async fn prepare_pack(
    state: &AppState,
    spec: PackPrepareSpec,
) -> Result<(Instance, String), String> {
    let pack_path = spec.pack_path;
    let (pack_name, mc_version, loader, loader_version) = detect(&pack_path)
        .await
        .map_err(|e| format!("解析整合包失败: {e}（文件: {}）", pack_path.display()))?;
    let mut instance = crate::instances::create_instance(
        state,
        pack_name.clone(),
        mc_version,
        loader,
        if loader_version.is_empty() { None } else { Some(loader_version) },
    )?;
    let instance_dir = state.instances_dir().join(&instance.id);

    // 图标：优先用包内图标，其次用项目图标兜底。
    // 注意要同时更新返回值，否则调用方拿到的实例缺 icon。
    let mut icon_path = crate::util::extract_modpack_icon(&pack_path, &instance_dir);
    if icon_path.is_none() {
        if let Some(u) = spec.icon_fallback_url.as_deref().filter(|s| !s.is_empty()) {
            icon_path = crate::util::download_icon(&state.client, u, &instance_dir).await;
        }
    }
    if let Some(icon_path) = icon_path {
        instance.icon = Some(format!("img:{icon_path}"));
        let _ = crate::util::log_best_effort("save_instance", crate::instances::save_instance(state, &instance));
    }

    Ok((instance, format!("整合包：{pack_name}")))
}

/// 第二步：下载内容 → 登记记录 → 解压 overrides → 安装游戏本体 → 标记已安装。
/// 注意：游戏本体安装失败会返回 Err（实例与内容文件此时已就位，
/// 用户仍可在实例页手动「安装游戏本体」）。
pub struct PackContentsSpec {
    /// 任务 id（下载中心的进度卡片）
    pub task_id: u64,
    /// 进度来源文案（由 prepare_pack 返回）
    pub source: String,
    /// 待下载的内容文件（已含目标路径 / hash / 大小）
    pub downloads: Vec<DownloadItem>,
    /// 下载阶段的文案，如「正在下载 187 个模组…」
    pub download_label: String,
    /// 预先构造好的内容记录：`(kind, record)`，kind 为 mod/resourcepack/shader。
    /// 为空时骨架会在下载完成后**扫描磁盘**自动登记（并可用 `hash_meta` 补来源）。
    pub records: Vec<(String, InstalledContent)>,
    /// 文件名 → (project_id, version_id)，用于扫盘登记时补上在线来源信息
    pub hash_meta: std::collections::HashMap<String, (String, String)>,
    /// zip 内需要跳过的包元数据前缀（如 ["modrinth.index.json", "META-INF/"]）
    pub skip_prefixes: Vec<String>,
    /// overrides 目录在 zip 内的前缀（CF 与 Modrinth 均为 "overrides"）
    pub overrides_prefix: String,
}

/// 返回登记的内容条数（用于回传前端展示）。
pub async fn install_pack_contents(
    app: &tauri::AppHandle,
    state: &AppState,
    instance: &Instance,
    pack_path: &std::path::Path,
    spec: PackContentsSpec,
) -> Result<usize, String> {
    let source = spec.source.clone();
    let instance_dir = state.instances_dir().join(&instance.id);

    // 1) 下载整合包内容
    let file_count = spec.downloads.len();
    if file_count > 0 {
        crate::install::emit_progress(
            app,
            spec.task_id,
            "modpack",
            &spec.download_label,
            0,
            file_count,
            instance,
            &source,
        );
        crate::download::download_many(app.clone(), state, spec.task_id, "modpack", spec.downloads).await?;
    }

    // 2) 登记内容记录：优先用调用方给的精确记录；否则扫磁盘自动登记
    let mut registered = 0usize;
    if spec.records.is_empty() {
        registered = scan_mod_records(state, &instance.id, &spec.hash_meta)?;
    } else {
        let mut by_kind: std::collections::HashMap<String, Vec<InstalledContent>> =
            std::collections::HashMap::new();
        for (kind, mut rec) in spec.records {
            // 文件此刻已下载到位，补全 jar 内元数据（mod_id / 作者 / 图标等）
            let folder = crate::modrinth::kind_folder(&kind);
            let path = instance_dir.join(folder).join(&rec.filename);
            crate::util::fill_content_from_jar(&mut rec, &path);
            registered += 1;
            by_kind.entry(kind).or_default().push(rec);
        }
        for (kind, records) in by_kind {
            add_content_batch(state, &instance.id, &kind, records)?;
        }
    }

    // 3) 解压 overrides（去掉前缀铺到实例根）与包内其它文件
    crate::install::emit_progress(
        app,
        spec.task_id,
        "modpack-install",
        "正在写入整合包文件…",
        0,
        1,
        instance,
        &source,
    );
    let overrides_dir_rel = format!("{}/", spec.overrides_prefix);
    let mut skip: Vec<String> = spec.skip_prefixes.clone();
    skip.push(overrides_dir_rel.clone());
    let skip_refs: Vec<&str> = skip.iter().map(|s| s.as_str()).collect();
    crate::util::extract_zip(pack_path, &instance_dir, &skip_refs)
        .map_err(|e| format!("解压整合包失败: {e}"))?;
    let strip_skip: Vec<&str> = spec.skip_prefixes.iter().map(|s| s.as_str()).collect();
    crate::util::extract_zip_strip(pack_path, &instance_dir, &overrides_dir_rel, &strip_skip)?;
    let overrides_dir = instance_dir.join(&spec.overrides_prefix);
    crate::util::fs_best_effort("remove_dir_all", &overrides_dir, std::fs::remove_dir_all(&overrides_dir));
    crate::install::emit_progress(
        app,
        spec.task_id,
        "modpack-install",
        "整合包文件已写入",
        1,
        1,
        instance,
        &source,
    );

    // 4) 安装游戏本体并标记完成
    crate::install::install_game(app.clone(), state, instance)
        .await
        .map_err(|e| format!("游戏文件安装失败：{e}"))?;
    let _ = crate::util::log_best_effort("mark_installed", crate::instances::mark_installed(state, &instance.id));

    Ok(registered)
}

/// Read the pack metadata (name, mc version, loader) from a local
/// `.mrpack` or CurseForge modpack zip.
pub async fn detect(path: &std::path::Path) -> Result<(String, String, LoaderType, String), String> {
    if let Ok(bytes) = crate::util::read_zip_entry(path, "modrinth.index.json") {
        let index: Value = serde_json::from_slice(&bytes).map_err(|e| format!("modrinth.index.json 解析失败: {e}"))?;
        let name = index
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("导入的整合包")
            .to_string();
        let mc = index
            .get("dependencies")
            .and_then(|d| d.get("minecraft"))
            .and_then(|m| m.as_str())
            .ok_or("缺少 minecraft 版本依赖")?
            .to_string();
        let (loader, lv) = detect_mrpack_loader(&index);
        Ok((name, mc, loader, lv))
    } else if let Ok(bytes) = crate::util::read_zip_entry(path, "manifest.json") {
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|e| format!("manifest.json 解析失败: {e}"))?;
        let name = manifest
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("导入的整合包")
            .to_string();
        let mc = manifest
            .get("minecraft")
            .and_then(|m| m.get("version"))
            .and_then(|v| v.as_str())
            .ok_or("缺少 minecraft 版本")?
            .to_string();
        let loader_id = manifest
            .get("minecraft")
            .and_then(|m| m.get("modLoaders"))
            .and_then(|l| l.as_array())
            .and_then(|a| a.first())
            .and_then(|f| f.get("id"))
            .and_then(|i| i.as_str())
            .unwrap_or("");
        let (loader, lv) = parse_cf_loader(loader_id);
        Ok((name, mc, loader, lv))
    } else {
        Err("无法识别的整合包格式（需要 modrinth.index.json 或 manifest.json）".into())
    }
}

fn detect_mrpack_loader(index: &Value) -> (LoaderType, String) {
    let deps = &index["dependencies"];
    for (key, lt) in [
        ("fabric-loader", LoaderType::Fabric),
        ("quilt-loader", LoaderType::Quilt),
        ("neoforge", LoaderType::NeoForge),
        ("forge", LoaderType::Forge),
    ] {
        if let Some(v) = deps.get(key).and_then(|v| v.as_str()) {
            return (lt, v.to_string());
        }
    }
    (LoaderType::Vanilla, String::new())
}

fn parse_cf_loader(id: &str) -> (LoaderType, String) {
    for (prefix, lt) in [
        ("fabric-", LoaderType::Fabric),
        ("quilt-", LoaderType::Quilt),
        ("neoforge-", LoaderType::NeoForge),
        ("forge-", LoaderType::Forge),
    ] {
        if let Some(rest) = id.strip_prefix(prefix) {
            return (lt, rest.to_string());
        }
    }
    (LoaderType::Vanilla, id.to_string())
}

/// 从本地整合包文件完整安装：解析包索引 → 下载内容 → 登记记录 → 解压 overrides →
/// 安装游戏本体 → 标记已安装（与在线安装 Modrinth / CurseForge 共用同一骨架
/// [`install_pack_contents`]）。
///
/// 支持两种包格式（二选一）：
/// - Modrinth `.mrpack`：按 `modrinth.index.json` 的 `files[]` 下载地址拉取模组等，
///   `overrides/` 解压到实例根；
/// - CurseForge `.zip`：按 `manifest.json` 的 `files[]` (projectID, fileID) 逐个查
///   元数据后下载。
///
/// 进度与成败都通过 `install://progress` 事件（`task_id`）上报给下载中心。
pub async fn install_local_pack(
    app: &tauri::AppHandle,
    state: &AppState,
    task_id: u64,
    instance: &Instance,
    pack_path: &std::path::Path,
) -> Result<(), String> {
    let source = format!("整合包：{}", instance.name);
    let result = install_local_pack_inner(app, state, task_id, instance, pack_path, &source).await;
    match &result {
        Ok(()) => {
            crate::install::emit_progress(app, task_id, "done", "整合包安装完成", 1, 1, instance, &source);
        }
        Err(e) => {
            let _ = app.emit(
                "install://progress",
                serde_json::json!({
                    "taskId": task_id,
                    "stage": "done",
                    "message": format!("安装失败：{e}"),
                    "done": 0,
                    "total": 0,
                    "instanceId": instance.id,
                    "instanceName": instance.name,
                    "source": source,
                    "ok": false,
                }),
            );
        }
    }
    result
}

async fn install_local_pack_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    task_id: u64,
    instance: &Instance,
    pack_path: &std::path::Path,
    source: &str,
) -> Result<(), String> {
    let instance_dir = state.instances_dir().join(&instance.id);
    std::fs::create_dir_all(&instance_dir).map_err(|e| e.to_string())?;

    // ---- Modrinth .mrpack ----
    if let Ok(bytes) = crate::util::read_zip_entry(pack_path, "modrinth.index.json") {
        let index: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("modrinth.index.json 解析失败: {e}"))?;
        let (items, hash_by_name) = crate::modrinth::parse_index_files(&index, &instance_dir)?;
        // 按文件哈希批量反查 Project/Version，供扫盘登记时补上在线来源信息
        let resolved = crate::modrinth::resolve_by_hashes(
            state,
            &hash_by_name.values().cloned().collect::<Vec<_>>(),
        )
        .await;
        let mut hash_meta: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        for (fname, h) in &hash_by_name {
            if let Some((pid, vid)) = resolved.get(h) {
                hash_meta.insert(fname.clone(), (pid.clone(), vid.clone()));
            }
        }
        let overrides = index
            .get("overrides")
            .and_then(|v| v.as_str())
            .unwrap_or("overrides")
            .to_string();
        let file_count = items.len();
        install_pack_contents(
            app,
            state,
            instance,
            pack_path,
            PackContentsSpec {
                task_id,
                source: source.to_string(),
                download_label: format!("正在下载 {file_count} 个文件…"),
                downloads: items,
                records: Vec::new(),
                hash_meta,
                skip_prefixes: vec!["modrinth.index.json".into(), "META-INF/".into()],
                overrides_prefix: overrides,
            },
        )
        .await?;
        return Ok(());
    }

    // ---- CurseForge manifest.json ----
    if let Ok(bytes) = crate::util::read_zip_entry(pack_path, "manifest.json") {
        let manifest: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("manifest.json 解析失败: {e}"))?;
        let pairs = crate::curseforge::manifest_file_pairs(&manifest);
        let total = pairs.len();
        let mut metas: Vec<(u64, u64, Value)> = Vec::with_capacity(total);
        for (idx, (pid, fid)) in pairs.iter().enumerate() {
            crate::install::emit_progress(
                app,
                task_id,
                "modpack",
                &format!("正在获取模组信息…（{}/{total}）", idx + 1),
                idx + 1,
                total,
                instance,
                source,
            );
            if let Ok(fbody) = crate::curseforge::file_info(state, *pid, *fid).await {
                if let Some(fdata) = fbody.get("data") {
                    metas.push((*pid, *fid, fdata.clone()));
                }
            }
        }
        let mods_dir = state.instances_dir().join(&instance.id).join("mods");
        let (dl_items, mod_records) = crate::curseforge::build_manifest_contents(&metas, &mods_dir);
        install_pack_contents(
            app,
            state,
            instance,
            pack_path,
            PackContentsSpec {
                task_id,
                source: source.to_string(),
                download_label: format!("正在下载 {} 个模组…", dl_items.len()),
                downloads: dl_items,
                records: mod_records
                    .into_iter()
                    .map(|r| ("mod".to_string(), r))
                    .collect(),
                hash_meta: Default::default(),
                skip_prefixes: vec!["manifest.json".into(), "META-INF/".into()],
                overrides_prefix: "overrides".into(),
            },
        )
        .await?;
        return Ok(());
    }

    Err("无法识别的整合包格式（需要 modrinth.index.json 或 manifest.json）".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn test_state(root: &std::path::Path) -> AppState {
        use std::sync::{Arc, Mutex, RwLock};
        AppState {
            root: root.to_path_buf(),
            settings: RwLock::new(Default::default()),
            db: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
            client: crate::settings::http_client("system", None),
            semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            game_pids: Arc::new(Mutex::new(std::collections::HashMap::new())),
            server_pids: Arc::new(Mutex::new(std::collections::HashMap::new())),
            server_senders: Arc::new(Mutex::new(std::collections::HashMap::new())),
            task_counter: std::sync::atomic::AtomicU64::new(1),
            install_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ms_flow: Arc::new(Mutex::new(None)),
            shimu: Mutex::new(crate::shimu::ShimuAuth::default()),
            ygg_oauth: Mutex::new(crate::yggdrasil_oauth::YggOauthPending::default()),
            java_cache: Mutex::new(None),
            terracotta: Mutex::new(None),
            pending_update: Mutex::new(None),
            lan_sync: Mutex::new(None),
        }
    }

    fn fresh(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 造一个 mrpack：modrinth.index.json（可选图标与 overrides 文件）
    fn write_mrpack(path: &std::path::Path, name: &str, with_icon: bool, files: Vec<(&str, &[u8])>) {
        let f = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        let index = serde_json::json!({
            "formatVersion": 1,
            "name": name,
            "dependencies": { "minecraft": "1.20.1", "fabric-loader": "0.15.11" },
            "files": []
        });
        zw.start_file("modrinth.index.json", opts).unwrap();
        zw.write_all(index.to_string().as_bytes()).unwrap();

        if with_icon {
            zw.start_file("pack.png", opts).unwrap();
            // ≥8 字节，满足图标提取的最小长度要求
            zw.write_all(&[0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01]).unwrap();
        }
        for (p, data) in files {
            zw.start_file(p, opts).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    /// 骨架第一步：从 mrpack 建实例，元信息与图标都要正确
    #[tokio::test]
    async fn prepare_pack_creates_instance_with_metadata_and_icon() {
        let root = fresh("qookix-pack-prepare");
        let state = test_state(&root);
        let pack = root.join("test.mrpack");
        write_mrpack(&pack, "骨架测试包", true, vec![("overrides/config/a.toml", b"x = 1")]);

        let (inst, source) = prepare_pack(
            &state,
            PackPrepareSpec { pack_path: pack.clone(), icon_fallback_url: None },
        )
        .await
        .unwrap();

        assert_eq!(inst.name, "骨架测试包");
        assert_eq!(inst.mc_version, "1.20.1");
        assert_eq!(inst.loader, LoaderType::Fabric);
        assert_eq!(inst.loader_version.as_deref(), Some("0.15.11"));
        assert_eq!(source, "整合包：骨架测试包");

        // 图标应被提取到实例目录
        assert!(inst.icon.is_some(), "包内 pack.png 应被提取为实例图标");
        let icon_path = state.instances_dir().join(&inst.id).join("pack-icon.png");
        assert!(icon_path.is_file(), "图标文件未落盘: {icon_path:?}");

        // 实例记录已持久化，可被后续步骤读到
        let saved = crate::instances::get_instance(&state, &inst.id).unwrap();
        assert_eq!(saved.name, "骨架测试包");
        assert_eq!(saved.loader, LoaderType::Fabric);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 包内无图标且没有兜底 URL 时不应报错，也没有图标
    #[tokio::test]
    async fn prepare_pack_without_icon_is_fine() {
        let root = fresh("qookix-pack-prepare-noicon");
        let state = test_state(&root);
        let pack = root.join("noicon.mrpack");
        write_mrpack(&pack, "无图标包", false, vec![]);

        let (inst, _) = prepare_pack(
            &state,
            PackPrepareSpec { pack_path: pack.clone(), icon_fallback_url: None },
        )
        .await
        .unwrap();
        assert!(inst.icon.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 非整合包（既无 index 也无 manifest）应给出明确错误而不是建出空实例
    #[tokio::test]
    async fn prepare_pack_rejects_unknown_format() {
        let root = fresh("qookix-pack-badformat");
        let state = test_state(&root);
        let pack = root.join("garbage.zip");
        {
            let f = std::fs::File::create(&pack).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            zw.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
            zw.write_all(b"not a modpack").unwrap();
            zw.finish().unwrap();
        }
        let r = prepare_pack(
            &state,
            PackPrepareSpec { pack_path: pack.clone(), icon_fallback_url: None },
        )
        .await;
        assert!(r.is_err(), "无法识别的包应报错");
        // 不应留下半个实例
        let count = std::fs::read_dir(state.instances_dir())
            .map(|rd| rd.flatten().count())
            .unwrap_or(0);
        assert_eq!(count, 0, "解析失败不应创建实例");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 扫盘登记：只登记 .jar，并用 hash_meta 补上在线来源
    #[test]
    fn scan_mod_records_registers_jars_with_hash_meta() {
        let root = fresh("qookix-pack-scan");
        let state = test_state(&root);
        let inst = crate::instances::create_instance(
            &state,
            "扫盘测试".into(),
            "1.20.1".into(),
            LoaderType::Fabric,
            None,
        )
        .unwrap();
        let mods_dir = state.instances_dir().join(&inst.id).join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::write(mods_dir.join("sodium.jar"), b"fake-jar").unwrap();
        std::fs::write(mods_dir.join("unresolved.jar"), b"fake-jar-2").unwrap();
        std::fs::write(mods_dir.join("notes.txt"), b"not a jar").unwrap();

        let mut hash_meta = std::collections::HashMap::new();
        hash_meta.insert("sodium.jar".to_string(), ("AANobbMI".to_string(), "ver-1".to_string()));

        let n = scan_mod_records(&state, &inst.id, &hash_meta).unwrap();
        assert_eq!(n, 2, "只有 .jar 会被登记（txt 忽略）");

        let saved = crate::instances::get_instance(&state, &inst.id).unwrap();
        assert_eq!(saved.mods.len(), 2);
        let sodium = saved.mods.iter().find(|m| m.filename == "sodium.jar").unwrap();
        assert_eq!(sodium.project_id.as_deref(), Some("AANobbMI"), "哈希映射未生效");
        assert_eq!(sodium.version_id.as_deref(), Some("ver-1"));
        assert!(sodium.enabled);
        let unresolved = saved.mods.iter().find(|m| m.filename == "unresolved.jar").unwrap();
        assert!(unresolved.project_id.is_none(), "无哈希映射的 mod 不应有来源 id");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 没有 mods 目录时扫盘应返回 0 而不是报错
    #[test]
    fn scan_mod_records_tolerates_missing_mods_dir() {
        let root = fresh("qookix-pack-scan-empty");
        let state = test_state(&root);
        let inst = crate::instances::create_instance(
            &state,
            "空实例".into(),
            "1.20.1".into(),
            LoaderType::Vanilla,
            None,
        )
        .unwrap();
        let n = scan_mod_records(&state, &inst.id, &Default::default()).unwrap();
        assert_eq!(n, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
