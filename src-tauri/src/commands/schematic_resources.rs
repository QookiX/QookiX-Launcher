//! 从本地 MC 客户端 jar 提取投影预览所需资源（方块模型、方块状态、纹理图集）。
//! 扫描 .minecraft/versions/ 取最新 release 版本，从 jar 中提取并生成资源文件，按版本缓存。
#![allow(non_snake_case)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Cursor, Read};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use zip::ZipArchive;

/// 解析版本 ID（如 "1.21.4"）为可比较的数字元组。
fn parse_version_id(id: &str) -> Vec<u32> {
    id.split('.')
        .filter_map(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

/// "1.21.4" / "1.20" / "26.2" 这种像版本号的字符串。
/// 注意：整合包的版本目录名和 id 往往是哈希（如 `5046802d`），不算版本号。
fn looks_like_mc_version(s: &str) -> bool {
    let core = s.split('-').next().unwrap_or(s);
    core.contains('.') && core.chars().all(|c| c.is_ascii_digit() || c == '.') && core != "."
}

/// 读 jar 内部的 `version.json` 拿版本号（官方客户端 jar 里有；
/// Forge 的形如 "1.12.2-forge-14.23.5.2860"，取第一段）。
fn jar_version_id(jar_path: &Path) -> Option<String> {
    let file = File::open(jar_path).ok()?;
    let mut archive = ZipArchive::new(BufReader::new(file)).ok()?;
    let mut entry = archive.by_name("version.json").ok()?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf).ok()?;
    let v: Value = serde_json::from_str(&buf).ok()?;
    let id = v.get("id").and_then(|x| x.as_str())?;
    let core = id.split('-').next().unwrap_or(id);
    looks_like_mc_version(core).then(|| core.to_string())
}

/// 扫描 versions/ 目录，找**最新且资源格式可用**的版本。
///
/// 两个坑：
/// 1. 整合包版本的目录名 / `id` 常是哈希，`parse_version_id` 会解析出空值 ——
///    直接按 id 排序等于随机挑一个。真实版本要从 `inheritsFrom` 或 jar 里的
///    `version.json` 拿。
/// 2. 投影预览用的是 deepslate，只认 1.13+ 的扁平化资源（`block/stone`）。
///    1.12 及更早是 `blocks/anvil_base` 这种旧命名，提出来也对不上，必须跳过。
fn find_latest_version(versions_dir: &Path) -> Result<(String, PathBuf), String> {
    let entries = std::fs::read_dir(versions_dir).map_err(|e| format!("读取版本目录失败: {e}"))?;
    // 排序键：(是否经典 1.x, 版本号)。渲染用的 deepslate 跟的是经典版本号，
    // 新年份版本（26.2 这类）先让位，只有没装 1.x 时才用它。
    let mut best: Option<((u8, Vec<u32>), String, PathBuf)> = None;
    let mut legacy_only = false;

    for entry in entries.flatten() {
        let dir = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let jar_path = dir.join(format!("{name}.jar"));
        if !jar_path.is_file() {
            continue;
        }

        // 真实 MC 版本：inheritsFrom（模组版） > id（官方版） > jar 内 version.json
        let mut version = None;
        if let Ok(content) = std::fs::read_to_string(dir.join(format!("{name}.json"))) {
            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                version = v
                    .get("inheritsFrom")
                    .and_then(|x| x.as_str())
                    .or_else(|| v.get("id").and_then(|x| x.as_str()))
                    .filter(|s| looks_like_mc_version(s))
                    .map(|s| s.to_string());
            }
        }
        let Some(version) = version.or_else(|| jar_version_id(&jar_path)) else {
            continue;
        };
        let key = parse_version_id(&version);
        if key.is_empty() {
            continue;
        }
        if key < vec![1, 13] {
            legacy_only = true;
            continue;
        }
        let rank = (u8::from(key.first() == Some(&1)), key);
        if best.as_ref().is_none_or(|(bk, _, _)| rank > *bk) {
            best = Some((rank, version, jar_path));
        }
    }

    match best {
        Some((_, version, jar)) => Ok((version, jar)),
        None if legacy_only => Err(
            "投影预览需要 1.13 及以上的客户端资源，当前只找到更旧的版本，请先安装一个 1.13+ 的版本"
                .into(),
        ),
        None => Err("未找到可用的 MC 版本（需要 1.13+）".into()),
    }
}

/// 资源缓存格式版本：提取逻辑变化（新增索引 / 换图集布局 / 改填充）时递增。
///
/// ⚠️ 写入与校验必须用这同一个常量：此前写入端写 `v4-trns-alpha`、校验端读
/// `v7-atlas-pad`，两边对不上 → `is_cache_complete` 永远是 false，缓存目录每次
/// 都被删掉重新解包客户端 jar（打开投影白等几秒）。
///
/// `v8-alpha-expand`：解码改成 EXPAND+STRIP_16，灰度/tRNS 贴图终于带上 alpha，
/// 图集内容变了 —— 旧缓存必须作废重建。
///
/// `v9-anim-first-frame`：动画贴图（灯笼/水/岩浆/火…）只打包首帧，图集尺寸也变了。
const RESOURCE_CACHE_VERSION: &str = "v9-anim-first-frame";

/// 检查缓存目录是否已有完整资源。
fn is_cache_complete(cache_dir: &Path) -> bool {
    let version_file = cache_dir.join("cache-version.txt");
    match std::fs::read_to_string(&version_file) {
        Ok(v) if v.trim() == RESOURCE_CACHE_VERSION => {}
        _ => return false,
    }
    let files: &[(&str, u64)] = &[
        ("block-model-index.json", 1_000),
        ("block-state-index.json", 1_000),
        ("texture-layout.json", 1_000),
        ("texture-atlas.png", 10_000),
        ("block-property-defaults.json", 10),
        ("block-name-index.json", 10),
        ("_debug.json", 10),
    ];
    files.iter().all(|(f, min_size)| {
        let path = cache_dir.join(f);
        std::fs::metadata(&path).map(|m| m.len() >= *min_size).unwrap_or(false)
    })
}

struct TextureEntry {
    key: String,
    width: u32,
    height: u32,
    data: Vec<u8>,
}

/// 从 jar 提取所有资源并写入缓存目录。
fn extract_resources_from_jar(jar_path: &Path, cache_dir: &Path) -> Result<(), String> {
    let file = File::open(jar_path).map_err(|e| format!("打开 jar 失败: {e}"))?;
    let mut archive = ZipArchive::new(BufReader::new(file)).map_err(|e| format!("读取 jar 失败: {e}"))?;

    let mut model_index: BTreeMap<String, Value> = BTreeMap::new();
    let mut state_index: BTreeMap<String, Value> = BTreeMap::new();
    let mut textures: Vec<TextureEntry> = Vec::new();
    let mut lang_data: Option<Value> = None;
    // 带动画的贴图（有 `*.png.mcmeta` 且含 `animation`）：这些 PNG 是竖着叠帧的
    // 长条（灯笼 16×48、水 16×512、岩浆 32×1024…），打包图集前要裁成首帧。
    let mut animated: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    let mut tex_fail = 0;
    let mut tex_fail_msg = String::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 jar 条目失败: {e}"))?;
        let name = entry.name().to_string();

        if name.ends_with('/') {
            continue;
        }

        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("读取 jar 内容失败: {e}"))?;

        if name.starts_with("assets/minecraft/models/block/") && name.ends_with(".json") {
            let key = name
                .strip_prefix("assets/minecraft/models/")
                .and_then(|s| s.strip_suffix(".json"))
                .unwrap_or("")
                .to_string();
            if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                model_index.insert(key, v);
            }
        } else if name.starts_with("assets/minecraft/blockstates/") && name.ends_with(".json") {
            let key = name
                .strip_prefix("assets/minecraft/blockstates/")
                .and_then(|s| s.strip_suffix(".json"))
                .unwrap_or("")
                .to_string();
            if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                state_index.insert(key, v);
            }
        } else if name.starts_with("assets/minecraft/textures/") && name.ends_with(".png") {
            let key = name
                .strip_prefix("assets/minecraft/textures/")
                .and_then(|s| s.strip_suffix(".png"))
                .unwrap_or("")
                .to_string();
            match decode_png(&buf) {
                Ok(tex) => {
                    textures.push(TextureEntry { key, width: tex.0, height: tex.1, data: tex.2 });
                }
                Err(e) if tex_fail < 3 => {
                    tex_fail += 1;
                    tex_fail_msg.push_str(&format!("{}: {} | ", key, e));
                }
                Err(_) => { tex_fail += 1; }
            }
        } else if name.starts_with("assets/minecraft/textures/") && name.ends_with(".png.mcmeta") {
            // 动画元数据：`{"animation": {...}}` 才是叠帧长条；只有 `{"gui": ...}`
            // 的那批（gui 缩放信息）不算，不能裁。
            let key = name
                .strip_prefix("assets/minecraft/textures/")
                .and_then(|s| s.strip_suffix(".png.mcmeta"))
                .unwrap_or("")
                .to_string();
            if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                if v.get("animation").is_some() {
                    animated.insert(key);
                }
            }
        } else if name == "assets/minecraft/lang/en_us.json" {
            if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                lang_data = Some(v);
            }
        }
    }

    // 动画贴图只保留第一帧 —— 图集里存整条 16×512 的话，模型 UV 0..16 会把
    // 全部帧一起铺到一个面上，看起来就是一条条横着的色带（用户报的"灯笼贴图错乱"）。
    // 静态预览不需要动画，首帧就是最直观的样子。
    let mut cropped = 0;
    for tex in textures.iter_mut() {
        if !animated.contains(&tex.key) {
            continue;
        }
        if let Some(new_height) =
            first_frame_height(tex.width, tex.height, tex.data.len())
        {
            tex.data.truncate((tex.width as usize) * (new_height as usize) * 4);
            tex.height = new_height;
            cropped += 1;
        }
    }

    eprintln!(
        "[schematic] 模型:{} 状态:{} 纹理:{} 失败:{} 动画裁帧:{}",
        model_index.len(), state_index.len(), textures.len(), tex_fail, cropped
    );
    if !tex_fail_msg.is_empty() {
        eprintln!("[schematic] 失败示例: {}", tex_fail_msg);
    }

    // 防呆：渲染器（deepslate）只认 1.13+ 的扁平化模型。旧格式（`blocks/anvil_base`
    // 这种）提出来两边键名对不上，所有方块都会退化到图集左上角一个像素，
    // 整张投影变成一坨同色块 —— 不如直接报错说清楚。
    if !model_index.contains_key("block/stone") {
        return Err(
            "该版本的资源是 1.13 之前的旧格式，投影预览需要 1.13 及以上的客户端资源".to_string(),
        );
    }

    std::fs::create_dir_all(cache_dir).map_err(|e| format!("创建缓存目录失败: {e}"))?;

    // _debug.json
    let debug = json!({
        "models": model_index.len(),
        "states": state_index.len(),
        "textures_ok": textures.len(),
        "textures_fail": tex_fail,
        "fail_samples": tex_fail_msg,
    });
    std::fs::write(cache_dir.join("_debug.json"), debug.to_string())
        .map_err(|e| format!("写入调试信息失败: {e}"))?;

    // block-model-index.json
    let model_json = serde_json::to_string(&model_index)
        .map_err(|e| format!("序列化模型索引失败: {e}"))?;
    std::fs::write(cache_dir.join("block-model-index.json"), model_json)
        .map_err(|e| format!("写入模型索引失败: {e}"))?;

    // block-state-index.json
    let state_json = serde_json::to_string(&state_index)
        .map_err(|e| format!("序列化状态索引失败: {e}"))?;
    std::fs::write(cache_dir.join("block-state-index.json"), state_json)
        .map_err(|e| format!("写入状态索引失败: {e}"))?;

    // texture-atlas.png + texture-layout.json
    let (atlas_w, atlas_h, atlas_data, layout) = pack_texture_atlas(&textures);
    eprintln!("[schematic] 图集尺寸:{}x{} 布局条目:{}", atlas_w, atlas_h, layout.len());
    eprintln!("[schematic] 样例键:{}", {
        let samples: Vec<&str> = layout.keys().take(5).map(|s| s.as_str()).collect();
        samples.join(", ")
    });
    encode_png(cache_dir.join("texture-atlas.png"), atlas_w, atlas_h, &atlas_data)
        .map_err(|e| format!("写入纹理图集失败: {e}"))?;
    let layout_json = serde_json::to_string(&layout)
        .map_err(|e| format!("序列化纹理布局失败: {e}"))?;
    std::fs::write(cache_dir.join("texture-layout.json"), layout_json)
        .map_err(|e| format!("写入纹理布局失败: {e}"))?;

    // block-property-defaults.json
    let defaults = generate_property_defaults(&state_index);
    let defaults_json = serde_json::to_string(&defaults)
        .map_err(|e| format!("序列化属性默认值失败: {e}"))?;
    std::fs::write(cache_dir.join("block-property-defaults.json"), defaults_json)
        .map_err(|e| format!("写入属性默认值失败: {e}"))?;

    // block-name-index.json
    let names = generate_name_index(lang_data);
    let names_json = serde_json::to_string(&names)
        .map_err(|e| format!("序列化方块名称索引失败: {e}"))?;
    std::fs::write(cache_dir.join("block-name-index.json"), names_json)
        .map_err(|e| format!("写入方块名称索引失败: {e}"))?;

    std::fs::write(cache_dir.join("cache-version.txt"), RESOURCE_CACHE_VERSION)
        .map_err(|e| format!("写入缓存版本失败: {e}"))?;

    Ok(())
}

/// 解码 PNG 为 RGBA 像素数据。
///
/// 关键：必须让 png crate 先做 `EXPAND` + `STRIP_16`（即 `normalize_to_color8`）。
///
/// * `EXPAND`：调色板→RGB、<8bit 的灰度→8bit、**tRNS 色键→alpha 通道**；
/// * `STRIP_16`：16bit 采样→8bit。
///
/// 不展开会踩两个坑：
/// 1. 原版有一批「灰度 + tRNS」的贴图（睡莲、藤蔓、云杉树叶、铁砧顶面、
///    草方块侧面叠加层、萤石/瓜藤等，1.21.11 里共 17 张方块贴图）。旧代码
///    只看 `color_type == Grayscale`，把 tRNS 扔了 → 透明像素退化成**不透明黑块**，
///    预览里就是睡莲/藤蔓该透的地方一圈黑（用户报的正是这个）。
/// 2. 1/2/4bit 的灰度贴图（`effect/dither`、部分 gui sprites）按「1 字节 1 像素」
///    校验尺寸必然失败，整张贴图被丢弃。
fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16,
    );
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("PNG 解码失败: {e}"))?;

    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("PNG 帧读取失败: {e}"))?;

    let width = frame.width;
    let height = frame.height;
    let expected = (width as usize) * (height as usize);
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err("索引 PNG 未被展开".to_string()),
    };
    let src = &buf[..frame.buffer_size()];
    if src.len() < expected * channels {
        return Err(format!(
            "像素数据大小不匹配: {} < {}",
            src.len(),
            expected * channels
        ));
    }
    if channels == 4 {
        return Ok((width, height, src.to_vec()));
    }

    let mut rgba = Vec::with_capacity(expected * 4);
    for i in 0..expected {
        let b = i * channels;
        match channels {
            1 => {
                let g = src[b];
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            2 => {
                let g = src[b];
                rgba.extend_from_slice(&[g, g, g, src[b + 1]]);
            }
            _ => rgba.extend_from_slice(&[src[b], src[b + 1], src[b + 2], 255]),
        }
    }
    Ok((width, height, rgba))
}

/// 动画贴图长条的「首帧高度」。MC 的动画帧一律是 `宽×宽` 竖着叠起来，所以首帧
/// 高度就等于宽度；不是整数倍（或本来就是方形）就说明不是叠帧长条，返回 None。
fn first_frame_height(width: u32, height: u32, data_len: usize) -> Option<u32> {
    if width == 0 || height <= width || height % width != 0 {
        return None;
    }
    let frame_len = (width as usize) * (width as usize) * 4;
    (data_len >= frame_len).then_some(width)
}

/// 编码 RGBA 数据为 PNG 文件。
fn encode_png(path: PathBuf, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("创建 PNG 文件失败: {e}"))?;
    let w = BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("PNG 编码头失败: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("PNG 编码数据失败: {e}"))?;
    Ok(())
}

/// 将纹理打包成图集（shelf-based packing）。
fn pack_texture_atlas(
    textures: &[TextureEntry],
) -> (u32, u32, Vec<u8>, BTreeMap<String, [u32; 4]>) {
    // 宽一点、矮一点：1024 宽会堆出两万多像素高，超过不少显卡的
    // MAX_TEXTURE_SIZE（常见上限 16384），贴着上限很危险。
    const MAX_WIDTH: u32 = 4096;
    /// 每张贴图四周留的边：内容用边缘像素扩展填充。
    /// 必须留 —— 掠射角下 mipmap 会越出贴图边界采样，紧挨着打包就会把隔壁
    /// 贴图（很多是透明的）混进来，alpha 掉到 alphaTest 阈值以下被抠掉，
    /// 每条方块边界就出现一条断续的暗线。
    const PAD: u32 = 2;

    let mut sorted: Vec<&TextureEntry> = textures.iter().collect();
    sorted.sort_by(|a, b| b.height.cmp(&a.height).then(b.width.cmp(&a.width)));

    let mut layout: BTreeMap<String, [u32; 4]> = BTreeMap::new();
    let mut atlas_w: u32 = 0;
    let mut atlas_h: u32 = 0;
    let mut shelf_x: u32 = 0;
    let mut shelf_y: u32 = 0;
    let mut shelf_h: u32 = 0;

    for tex in &sorted {
        let bw = tex.width + PAD * 2;
        let bh = tex.height + PAD * 2;
        if shelf_x + bw > MAX_WIDTH {
            shelf_y += shelf_h;
            shelf_x = 0;
            shelf_h = 0;
        }
        // 布局里只记内圈（真正贴图）的坐标，四周的边不参与 UV
        layout.insert(
            tex.key.clone(),
            [shelf_x + PAD, shelf_y + PAD, tex.width, tex.height],
        );
        shelf_x += bw;
        if bh > shelf_h {
            shelf_h = bh;
        }
        if shelf_x > atlas_w {
            atlas_w = shelf_x;
        }
        if shelf_y + shelf_h > atlas_h {
            atlas_h = shelf_y + shelf_h;
        }
    }

    if atlas_w == 0 {
        atlas_w = 1;
    }
    if atlas_h == 0 {
        atlas_h = 1;
    }

    let mut atlas_data = vec![0u8; (atlas_w * atlas_h * 4) as usize];
    for tex in &sorted {
        let &[x, y, w, h] = &layout[&tex.key];
        let expected_len = (w * h * 4) as usize;
        if tex.data.len() < expected_len || w == 0 || h == 0 {
            continue;
        }
        // 连四周的边一起写：源坐标 clamp 到贴图内 → 边缘像素向外扩展
        for row in 0..(h + PAD * 2) {
            let sy = row.saturating_sub(PAD).min(h - 1);
            let dst_y = (y - PAD) + row;
            for col in 0..(w + PAD * 2) {
                let sx = col.saturating_sub(PAD).min(w - 1);
                let src = ((sy * tex.width + sx) * 4) as usize;
                let dst = ((dst_y * atlas_w + ((x - PAD) + col)) * 4) as usize;
                if src + 4 > tex.data.len() || dst + 4 > atlas_data.len() {
                    continue;
                }
                atlas_data[dst..dst + 4].copy_from_slice(&tex.data[src..src + 4]);
            }
        }
    }

    (atlas_w, atlas_h, atlas_data, layout)
}

/// 从 blockstate JSON 提取默认属性值。
fn generate_property_defaults(
    state_index: &BTreeMap<String, Value>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut defaults: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (name, state) in state_index {
        let mut props: BTreeMap<String, String> = BTreeMap::new();
        if let Some(variants) = state.get("variants").and_then(|v| v.as_object()) {
            for key in variants.keys() {
                for part in key.split(',') {
                    if let Some(eq) = part.find('=') {
                        let k = &part[..eq];
                        let v = &part[eq + 1..];
                        props.entry(k.to_string()).or_insert_with(|| v.to_string());
                    }
                }
            }
        }
        if let Some(multipart) = state.get("multipart").and_then(|v| v.as_array()) {
            for part in multipart {
                if let Some(when) = part.get("when").and_then(|w| w.as_object()) {
                    for (k, v) in when {
                        if let Some(s) = v.as_str() {
                            props.entry(k.clone()).or_insert_with(|| s.to_string());
                        }
                    }
                }
            }
        }
        if !props.is_empty() {
            defaults.insert(name.clone(), props);
        }
    }
    defaults
}

/// 从语言文件生成方块名称索引。
fn generate_name_index(lang_data: Option<Value>) -> Value {
    let mut result = BTreeMap::new();
    if let Some(lang) = lang_data.and_then(|v| v.as_object().map(|o| o.clone())) {
        let mut block_names = BTreeMap::new();
        for (k, v) in &lang {
            if k.starts_with("block.minecraft.") {
                if let Some(s) = v.as_str() {
                    block_names.insert(k.clone(), s.to_string());
                }
            }
        }
        result.insert("en_us".to_string(), block_names);
    }
    json!(result)
}

/// 提取是重活（解码几千张贴图 + 打包图集 + 编码 PNG）。同时打开两个投影文件
/// 会并发提取、互相删掉对方的缓存目录，结果就是两边都拿到半成品 —— 串行化。
static EXTRACT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 提取本地 MC 资源，返回缓存目录路径。
#[tauri::command]
pub async fn schematic_extract_resources(
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<String, String> {
    let versions_dir = state.versions_dir();
    let root = state.root.clone();

    tokio::task::spawn_blocking(move || {
        // 按真实 MC 版本命名缓存目录：同一个版本的不同整合包共用一份，不重复提取
        let (version, jar_path) = find_latest_version(&versions_dir)?;

        let cache_dir = root.join("schematic-resources").join(&version);

        // 串行化；拿到锁后再查一次缓存，排在前面的任务可能刚提完
        let _guard = EXTRACT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if is_cache_complete(&cache_dir) {
            return Ok(cache_dir.to_string_lossy().to_string());
        }

        if cache_dir.exists() {
            std::fs::remove_dir_all(&cache_dir)
                .map_err(|e| format!("清空旧缓存失败: {e}"))?;
        }

        extract_resources_from_jar(&jar_path, &cache_dir)?;

        Ok(cache_dir.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("资源提取任务失败: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_version(dir: &Path, name: &str, json: &str) {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{name}.jar")), b"stub").unwrap();
        std::fs::write(d.join(format!("{name}.json")), json).unwrap();
    }

    /// 整合包版本的目录名/id 是哈希，必须按 inheritsFrom 里的真实版本判定，
    /// 并且跳过 1.13 之前的旧资源格式
    #[test]
    fn picks_modern_version_not_modpack_hash() {
        let base = std::env::temp_dir().join(format!("qookix-ver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        // 1.12.2 整合包：目录名是哈希，真实版本写在 inheritsFrom 里
        write_version(
            &base,
            "5046802d",
            r#"{"id":"5046802d","type":"release","inheritsFrom":"1.12.2"}"#,
        );
        // 1.21.3：id 本身就是版本号
        write_version(&base, "a4b7bb8b", r#"{"id":"1.21.3","type":"release"}"#);
        // 年份版本号（渲染库跟的是经典 1.x，让它让位）
        write_version(&base, "c2d82696", r#"{"id":"26.2","type":"release"}"#);

        let (version, jar) = find_latest_version(&base).expect("应挑到 1.21.3");
        assert_eq!(version, "1.21.3");
        assert_eq!(jar, base.join("a4b7bb8b").join("a4b7bb8b.jar"));

        // 只剩年份版本时，它也是可用的（1.13+ 的扁平化资源）
        std::fs::remove_dir_all(base.join("a4b7bb8b")).unwrap();
        assert_eq!(find_latest_version(&base).unwrap().0, "26.2");
        std::fs::remove_dir_all(base.join("c2d82696")).unwrap();

        // 只剩旧版本：明确报错，而不是提一堆用不了的资源出来
        let err = find_latest_version(&base).unwrap_err();
        assert!(err.contains("1.13"), "错误信息要说明需要 1.13+：{err}");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 图集里每张贴图四周要有边缘扩展的边（否则 mip 采样会串到隔壁贴图）
    #[test]
    fn atlas_pads_tiles_with_edge_replication() {
        let mk = |key: &str, color: [u8; 4]| {
            let mut data = Vec::new();
            for _ in 0..4 {
                data.extend_from_slice(&color);
            }
            TextureEntry {
                key: key.to_string(),
                width: 2,
                height: 2,
                data,
            }
        };
        let (w, _h, data, layout) = pack_texture_atlas(&[
            mk("a", [255, 0, 0, 255]),
            mk("b", [0, 0, 255, 128]),
        ]);
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [data[i], data[i + 1], data[i + 2], data[i + 3]]
        };
        let [ax, ay, aw, ah] = layout["a"];
        assert_eq!(px(ax, ay), [255, 0, 0, 255], "内圈是原像素");
        assert_eq!(px(ax - 1, ay), [255, 0, 0, 255], "左边要做边缘扩展");
        assert_eq!(px(ax + aw, ay + ah - 1), [255, 0, 0, 255], "右边要做边缘扩展");
        assert_eq!(px(ax, ay + ah), [255, 0, 0, 255], "下边要做边缘扩展");
        assert_eq!(px(ax + aw - 1, ay - 1), [255, 0, 0, 255], "上边要做边缘扩展");

        let [bx, ..] = layout["b"];
        assert!(bx >= ax + aw + 2, "两张贴图之间要隔开：{ax}+{aw} vs {bx}");
    }

    /// 灰度 + tRNS 的贴图（睡莲、藤蔓、云杉树叶这一批）必须解出真正的 alpha，
    /// 否则 tRNS 被忽略、透明像素退化成不透明黑块
    #[test]
    fn decode_png_expands_grayscale_trns_to_alpha() {
        use base64::Engine;
        // 4×4 灰度 PNG，tRNS 声明灰度值 0 为透明色；左上 2×2 是 0，其余是 200
        const B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAAAAACMmsGiAAAAAnRSTlMAAHaTzTgAAAATSURBVHicY2BgOHGCAUycOAElAEsUCWE9Mt9cAAAAAElFTkSuQmCC";
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(B64)
            .expect("测试用 PNG 的 base64 应可解码");

        let (w, h, rgba) = decode_png(&bytes).expect("灰度 + tRNS 应能解码");
        assert_eq!((w, h), (4, 4));
        assert_eq!(rgba.len(), 4 * 4 * 4);
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0], "tRNS 声明的颜色必须变成全透明");
        let opaque = &rgba[rgba.len() - 4..];
        assert_eq!(opaque, &[200, 200, 200, 255], "其余像素应不透明");
        assert!(
            rgba.chunks_exact(4).filter(|p| p[3] == 0).count() == 4,
            "恰好 4 个像素透明"
        );
    }

    /// 动画贴图必须只用首帧：整条 16×48 打包进图集的话，模型 UV 0..16 会把
    /// 三帧一起铺到一个面上（灯笼那圈横条色带就是这么来的）
    #[test]
    fn animation_strip_uses_first_frame_only() {
        // 16×48 = 3 帧，每帧 16×16
        let mut data = vec![0u8; 16 * 48 * 4];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let h = first_frame_height(16, 48, data.len()).expect("16x48 应识别为 3 帧长条");
        assert_eq!(h, 16);
        data.truncate((16 * h as usize) * 4);
        assert_eq!(data.len(), 16 * 16 * 4, "只留第一帧");

        // 方形贴图不动
        assert_eq!(first_frame_height(16, 16, 16 * 16 * 4), None);
        // 高度不是宽度的整数倍：不是叠帧长条，不动
        assert_eq!(first_frame_height(16, 40, 16 * 40 * 4), None);
        // 32×1024 的流水（32 帧）
        assert_eq!(first_frame_height(32, 1024, 32 * 1024 * 4), Some(32));
    }

    #[test]
    fn looks_like_version() {
        assert!(looks_like_mc_version("1.21.4"));
        assert!(looks_like_mc_version("26.2"));
        assert!(!looks_like_mc_version("5046802d"));
        assert!(!looks_like_mc_version("a4b7bb8b"));
    }
}
