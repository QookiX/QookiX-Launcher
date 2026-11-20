//! 区块文件（`.mca` / Anvil region）读写。
//!
//! 文件结构：
//! - 8 KiB 头：1024 个 4 字节条目（高 3 字节 = 起始扇区号，低 1 字节 = 扇区数）
//!   + 1024 个 4 字节时间戳；条目顺序 = `(cx & 31) + (cz & 31) * 32`
//! - 数据区：`4 字节大端长度（含压缩类型字节） + 1 字节压缩类型 + NBT`，
//!   按 4 KiB 扇区对齐，尾部补零。压缩类型 1=gzip、2=zlib、3=不压缩。
//!
//! 写入在文件锁下流式复制到同目录的临时文件，更新区块后用重命名提交。
//! 新区块或放不下原槽位时
//! 追加到文件末尾并更新头部，旧槽位留空——游戏按头部表寻址，空洞无害。

use fastnbt::Value;
use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

const SECTOR: usize = 4096;
const HEADER_LEN: usize = SECTOR * 2;
/// 单个条目最多 255 个扇区（≈1 MiB），超过写不下
const MAX_SECTORS: usize = 255;
const MAX_CHUNK_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_REGION_BYTES: u64 = 2 * 1024 * 1024 * 1024;
type RegionLocks = HashMap<PathBuf, Weak<Mutex<()>>>;
static WRITE_LOCKS: LazyLock<Mutex<RegionLocks>> = LazyLock::new(Mutex::default);

/// 维度对应的 region 目录。
///
/// 两种布局都要认：
/// - 26.x（新的版本号方案）起，三个原版维度都搬到了
///   `dimensions/minecraft/{overworld,the_nether,the_end}/region`，存档根下
///   已经没有 `region/` 了；只认老路径的话这类存档的地图会整片空白、区块列表是 0。
/// - 更早的版本：主世界 `region/`，下界 `DIM-1/region`，末地 `DIM1/region`。
///
/// 谁存在用谁；都不存在时返回新布局的路径（新建区块按游戏当前的写法落盘）。
pub fn region_dir(world_dir: &Path, dim: &str) -> PathBuf {
    let (vanilla, legacy) = match dim {
        "nether" => ("the_nether", "DIM-1"),
        "end" => ("the_end", "DIM1"),
        _ => ("overworld", ""),
    };
    let modern = world_dir
        .join("dimensions")
        .join("minecraft")
        .join(vanilla)
        .join("region");
    if modern.is_dir() {
        return modern;
    }
    let old = if legacy.is_empty() {
        world_dir.join("region")
    } else {
        world_dir.join(legacy).join("region")
    };
    if old.is_dir() {
        return old;
    }
    // 两个都没生成过：按存档本身的布局来定（新存档写 dimensions 下）
    if world_dir.join("dimensions").is_dir() {
        modern
    } else {
        old
    }
}

/// 某个区块所在的 region 文件路径
pub fn region_file(world_dir: &Path, dim: &str, cx: i32, cz: i32) -> PathBuf {
    region_dir(world_dir, dim).join(format!("r.{}.{}.mca", cx >> 5, cz >> 5))
}

/// 条目下标：区块坐标在 region 内的相对位置
fn entry_index(cx: i32, cz: i32) -> usize {
    (cx.rem_euclid(32) + cz.rem_euclid(32) * 32) as usize
}

/// 读条目 → (起始字节偏移, 扇区数)；偏移 0 表示该区块未生成
fn read_entry(bytes: &[u8], index: usize) -> (u64, usize) {
    let p = index * 4;
    if bytes.len() < p + 4 {
        return (0, 0);
    }
    let raw = u32::from_be_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]);
    (u64::from(raw >> 8) * SECTOR as u64, (raw & 0xFF) as usize)
}

fn parse_region_name(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix("r.")?.strip_suffix(".mca")?;
    let (x, z) = rest.split_once('.')?;
    let (x, z) = (x.parse::<i32>().ok()?, z.parse::<i32>().ok()?);
    (x.checked_mul(512).is_some() && z.checked_mul(512).is_some()).then_some((x, z))
}

/// 列出该维度下所有已生成的区块（按坐标排序）
pub fn list_chunks(world_dir: &Path, dim: &str) -> Result<Vec<Json>, String> {
    let dir = region_dir(world_dir, dim);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).map_err(|e| format!("读取 region 目录失败: {e}"))?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some((rx, rz)) = parse_region_name(&name) else {
            continue;
        };
        // 只读 8KiB 头部：region 文件动辄几十 MB，整份读进来纯属浪费
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let Ok(mut f) = std::fs::File::open(path) else {
            continue;
        };
        let mut header = [0u8; HEADER_LEN];
        if f.read_exact(&mut header).is_err() {
            continue;
        }
        for i in 0..1024usize {
            let (offset, sectors) = read_entry(&header, i);
            if offset == 0 || sectors == 0 {
                continue;
            }
            let p = SECTOR + i * 4;
            let modified = u32::from_be_bytes([header[p], header[p + 1], header[p + 2], header[p + 3]]);
            out.push(json!({
                "cx": rx * 32 + (i % 32) as i32,
                "cz": rz * 32 + (i / 32) as i32,
                "size": sectors * SECTOR,
                "modified": modified,
            }));
        }
    }
    out.sort_by(|a, b| {
        let (ax, az) = (
            a.get("cx").and_then(|x| x.as_i64()).unwrap_or(0),
            a.get("cz").and_then(|x| x.as_i64()).unwrap_or(0),
        );
        let (bx, bz) = (
            b.get("cx").and_then(|x| x.as_i64()).unwrap_or(0),
            b.get("cz").and_then(|x| x.as_i64()).unwrap_or(0),
        );
        (az, ax).cmp(&(bz, bx))
    });
    Ok(out)
}

/// 读一个区块的 NBT（根复合节点就是区块数据，没有 level.dat 那种 Data 包装层）
pub fn read_chunk(world_dir: &Path, dim: &str, cx: i32, cz: i32) -> Result<Value, String> {
    let path = region_file(world_dir, dim, cx, cz);
    RegionReader::open(&path)
        .map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                format!("区域文件不存在（该区块尚未生成）：{}", path.display())
            } else {
                format!("打开区域文件 {} 失败: {err}", path.display())
            }
        })?
        .read_chunk(cx, cz)
}

pub(super) struct RegionReader {
    file: File,
    metadata: std::fs::Metadata,
    header: [u8; HEADER_LEN],
}

impl RegionReader {
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        if !std::fs::metadata(path)?.is_file() {
            return Err(invalid_data("区域文件必须是普通文件"));
        }
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_REGION_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "区域文件类型或长度无效"));
        }
        let mut reader = Self { file, metadata, header: [0; HEADER_LEN] };
        reader.file.read_exact(&mut reader.header)?;
        Ok(reader)
    }

    pub(super) fn read_chunk(&mut self, cx: i32, cz: i32) -> Result<Value, String> {
        let (offset, sectors) = read_entry(&self.header, entry_index(cx, cz));
        let size = sectors * SECTOR;
        if offset == 0 && size == 0 {
            return Err("该区块尚未生成".into());
        }
        if offset < HEADER_LEN as u64 || size == 0 || offset + size as u64 > self.metadata.len() {
            return Err(format!("区块 ({cx}, {cz}) 扇区超出区域文件范围"));
        }
        let mut slot = vec![0; size];
        self.file.seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut slot))
            .map_err(|err| format!("读取区块 ({cx}, {cz}) 失败: {err}"))?;
        decode_slot(&slot).map_err(|err| format!("区块 ({cx}, {cz}): {err}"))
    }

    pub(super) fn metadata(&self) -> &std::fs::Metadata {
        &self.metadata
    }

    #[cfg(windows)]
    pub(super) fn identity(&self) -> Option<(u64, u64)> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        let ok = unsafe { GetFileInformationByHandle(self.file.as_raw_handle(), info.as_mut_ptr()) };
        if ok == 0 {
            return None;
        }
        let info = unsafe { info.assume_init() };
        let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Some((u64::from(info.dwVolumeSerialNumber), index))
    }

    #[cfg(unix)]
    pub(super) fn identity(&self) -> Option<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        Some((self.metadata.dev(), self.metadata.ino()))
    }
}

fn decode_slot(slot: &[u8]) -> Result<Value, String> {
    if slot.len() < 5 {
        return Err("区块数据不完整".into());
    }
    let len = u32::from_be_bytes([slot[0], slot[1], slot[2], slot[3]]) as usize;
    // 长度包括压缩类型字节，实际 NBT 内容从类型之后开始。
    let (&ctype, payload) = slot[4..]
        .get(..len)
        .and_then(|chunk| chunk.split_first())
        .ok_or_else(|| "区块数据长度越界".to_string())?;
    let mut raw = Vec::new();
    match ctype {
        1 => GzDecoder::new(payload).take(MAX_CHUNK_BYTES as u64 + 1).read_to_end(&mut raw),
        2 => ZlibDecoder::new(payload).take(MAX_CHUNK_BYTES as u64 + 1).read_to_end(&mut raw),
        3 => return super::bounded::decode(payload, "区块"),
        4 => return Err("该区块用了 LZ4 压缩（模组格式），暂不支持".into()),
        other => return Err(format!("未知的区块压缩类型: {other}")),
    }.map_err(|err| format!("解压区块失败: {err}"))?;
    if raw.len() > MAX_CHUNK_BYTES {
        return Err("区块解压后超过 16 MiB 限制".into());
    }
    super::bounded::decode(&raw, "区块")
}

/// 在同一把区域文件锁下读、修改和写回区块。
pub fn edit_chunk(
    world_dir: &Path,
    dim: &str,
    cx: i32,
    cz: i32,
    edit: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    with_region_write(&region_file(world_dir, dim, cx, cz), |target, mut source| {
        let reader = source.as_mut().ok_or("区域文件不存在，该区块尚未生成")?;
        let mut data = reader.read_chunk(cx, cz)?;
        edit(&mut data)?;
        write_region(target, source, cx, cz, &data).map_err(|e| format!("写入区域文件失败: {e}"))
    })
}

#[cfg(test)]
pub(super) fn write_chunk(world_dir: &Path, dim: &str, cx: i32, cz: i32, data: &Value) -> Result<(), String> {
    with_region_write(&region_file(world_dir, dim, cx, cz), |target, source| {
        write_region(target, source, cx, cz, data).map_err(|e| format!("写入区域文件失败: {e}"))
    })
}

fn with_region_write<T>(
    path: &Path,
    action: impl FnOnce(&Path, Option<RegionReader>) -> Result<T, String>,
) -> Result<T, String> {
    let target = match path.canonicalize() {
        Ok(target) => target,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(format!("区域文件链接无效: {}", path.display())),
            }
            let parent = path.parent().ok_or("区域文件路径无效")?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            parent.canonicalize().map_err(|e| e.to_string())?
                .join(path.file_name().ok_or("区域文件名称无效")?)
        }
        Err(err) => return Err(format!("无法解析区域文件路径: {err}")),
    };
    let lock = {
        let mut locks = WRITE_LOCKS.lock().map_err(|_| "区域写入锁不可用")?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&target).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(target.clone(), Arc::downgrade(&lock));
            lock
        }
    };
    let _guard = lock.lock().map_err(|_| "区域写入锁不可用")?;
    let source = match RegionReader::open(&target) {
        Ok(reader) => Some(reader),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(format!("读取已有区域文件 {} 失败: {err}", target.display())),
    };
    action(&target, source)
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn validate_slots(header: &[u8], length: u64) -> io::Result<()> {
    let mut used = Vec::new();
    for index in 0..1024 {
        let (offset, sectors) = read_entry(header, index);
        if offset == 0 && sectors == 0 {
            continue;
        }
        let end = offset + (sectors * SECTOR) as u64;
        if offset < HEADER_LEN as u64 || sectors == 0 || end > length {
            return Err(invalid_data("已有区块位置损坏"));
        }
        used.push((offset, end));
    }
    used.sort_unstable();
    if used.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(invalid_data("区域文件中的区块位置重叠"));
    }
    Ok(())
}

fn write_region(
    target: &Path,
    source: Option<RegionReader>,
    cx: i32,
    cz: i32,
    data: &Value,
) -> io::Result<()> {
    let (mut header, length) = source.as_ref().map_or(
        ([0; HEADER_LEN], HEADER_LEN as u64),
        |reader| (reader.header, reader.metadata.len()),
    );
    validate_slots(&header, length)?;
    let raw = fastnbt::to_bytes_with_opts(data, fastnbt::SerOpts::new())
        .map_err(|err| invalid_data(format!("序列化区块失败: {err}")))?;
    if raw.len() > MAX_CHUNK_BYTES {
        return Err(invalid_data("区块原始数据超过 16 MiB 限制"));
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw)?;
    let payload = encoder.finish()?;

    let sectors = (5 + payload.len()).div_ceil(SECTOR);
    if sectors > MAX_SECTORS {
        return Err(invalid_data("区块数据过大，超出单个条目的上限"));
    }
    let index = entry_index(cx, cz);
    let (old_offset, old_sectors) = read_entry(&header, index);
    let offset = if old_offset != 0 && old_sectors >= sectors {
        old_offset
    } else {
        length.div_ceil(SECTOR as u64) * SECTOR as u64
    };
    let end = offset + (sectors * SECTOR) as u64;
    if end > MAX_REGION_BYTES {
        return Err(invalid_data("区域文件超过 2 GiB 限制"));
    }
    let parent = target.parent().ok_or_else(|| invalid_data("区域文件路径无效"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(reader) = source {
        let mut input = reader.file;
        input.seek(SeekFrom::Start(0))?;
        if io::copy(&mut input.take(length), temp.as_file_mut())? != length {
            return Err(invalid_data("复制过程中区域文件长度发生变化"));
        }
    }
    let entry = (((offset / SECTOR as u64) as u32) << 8) | sectors as u32;
    header[index * 4..index * 4 + 4].copy_from_slice(&entry.to_be_bytes());
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|age| age.as_secs() as u32).unwrap_or(0);
    let tp = SECTOR + index * 4;
    header[tp..tp + 4].copy_from_slice(&ts.to_be_bytes());
    let mut slot = vec![0; sectors * SECTOR];
    slot[..4].copy_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    slot[4] = 2;
    slot[5..5 + payload.len()].copy_from_slice(&payload);
    let output = temp.as_file_mut();
    output.seek(SeekFrom::Start(0))?;
    output.write_all(&header)?;
    output.seek(SeekFrom::Start(offset))?;
    output.write_all(&slot)?;
    output.sync_all()?;
    temp.persist(target).map_err(|err| err.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::level;
    use std::collections::HashMap;

    fn chunk_with(seed: &str) -> Value {
        let mut m = HashMap::new();
        m.insert("DataVersion".into(), Value::Int(3465));
        m.insert("xPos".into(), Value::Int(0));
        m.insert("zPos".into(), Value::Int(0));
        m.insert("Status".into(), Value::String("full".into()));
        m.insert("Note".into(), Value::String(seed.into()));
        Value::Compound(m)
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("qookix-region-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn decodes_chunk_that_fills_its_allocated_sector() {
        let mut chunk = chunk_with("");
        let empty = fastnbt::to_bytes(&chunk).unwrap();
        let note = "x".repeat(SECTOR - 5 - empty.len());
        if let Value::Compound(fields) = &mut chunk {
            fields.insert("Note".into(), Value::String(note.clone()));
        }
        let payload = fastnbt::to_bytes(&chunk).unwrap();
        assert_eq!(payload.len(), SECTOR - 5);
        let mut region = vec![0; HEADER_LEN + SECTOR];
        region[..4].copy_from_slice(&[0, 0, 2, 1]);
        let slot = &mut region[HEADER_LEN..];
        slot[..4].copy_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
        slot[4] = 3;
        slot[5..].copy_from_slice(&payload);

        let decoded = decode_slot(&region[HEADER_LEN..]).unwrap();
        assert_eq!(level::get_string(&decoded, "Note"), Some(note));
    }

    #[test]
    fn write_then_read_roundtrip() {
        let world = tmp_dir("roundtrip");
        write_chunk(&world, "overworld", 0, 0, &chunk_with("a")).unwrap();
        write_chunk(&world, "overworld", 33, -1, &chunk_with("b")).unwrap();

        let a = read_chunk(&world, "overworld", 0, 0).unwrap();
        assert_eq!(level::get_string(&a, "Note").as_deref(), Some("a"));
        let b = read_chunk(&world, "overworld", 33, -1).unwrap();
        assert_eq!(level::get_string(&b, "Note").as_deref(), Some("b"));
        assert_eq!(level::get_int(&b, "xPos"), Some(0));

        let list = list_chunks(&world, "overworld").unwrap();
        assert_eq!(list.len(), 2);
        // 两个区块落在同一个 region 文件里（33 与 -1 属于 r.1.-1）
        assert!(world.join("region/r.0.0.mca").is_file());
        assert!(world.join("region/r.1.-1.mca").is_file());
        let _ = std::fs::remove_dir_all(&world);
    }

    /// 放得下就原地覆盖：文件长度不变，后面的区块不受影响
    #[test]
    fn in_place_overwrite_keeps_file_tail() {
        let world = tmp_dir("inplace");
        write_chunk(&world, "overworld", 0, 0, &chunk_with("aaaaaaaa")).unwrap();
        write_chunk(&world, "overworld", 1, 0, &chunk_with("neighbour")).unwrap();
        let path = world.join("region/r.0.0.mca");
        let before = std::fs::metadata(&path).unwrap().len();

        // 改成更短的内容 → 仍是原地覆盖，绝不能把文件截断
        write_chunk(&world, "overworld", 0, 0, &chunk_with("b")).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            before,
            "原地覆盖不该改变文件长度"
        );
        let back = read_chunk(&world, "overworld", 0, 0).unwrap();
        assert_eq!(level::get_string(&back, "Note").as_deref(), Some("b"));
        let neighbour = read_chunk(&world, "overworld", 1, 0).unwrap();
        assert_eq!(
            level::get_string(&neighbour, "Note").as_deref(),
            Some("neighbour"),
            "原地覆盖不能切掉后面的区块"
        );
        let _ = std::fs::remove_dir_all(&world);
    }

    /// 放不下就迁移到文件末尾，隔壁区块同样必须完好
    #[test]
    fn larger_edit_relocates_and_keeps_neighbour() {
        let world = tmp_dir("grow");
        write_chunk(&world, "overworld", 0, 0, &chunk_with("keep-me")).unwrap();
        write_chunk(&world, "overworld", 1, 0, &chunk_with("x")).unwrap();
        let path = world.join("region/r.0.0.mca");
        let before = std::fs::metadata(&path).unwrap().len();

        // 用不可压缩的随机字节，确保放不下原槽位、必须迁移
        let mut x: u32 = 0x9E37_79B9;
        let noise: Vec<i8> = (0..20_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as i8
            })
            .collect();
        let mut big = chunk_with("grown");
        if let Value::Compound(m) = &mut big {
            m.insert("Noise".into(), Value::ByteArray(fastnbt::ByteArray::new(noise)));
        }
        write_chunk(&world, "overworld", 0, 0, &big).unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() > before,
            "放不下时应追加到文件末尾"
        );

        let grown = read_chunk(&world, "overworld", 0, 0).unwrap();
        assert_eq!(level::get_string(&grown, "Note").as_deref(), Some("grown"));
        assert!(level::get(&grown, "Noise").is_some());
        let neighbour = read_chunk(&world, "overworld", 1, 0).unwrap();
        assert_eq!(level::get_string(&neighbour, "Note").as_deref(), Some("x"));
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn missing_chunk_reports_clearly() {
        let world = tmp_dir("missing");
        let err = read_chunk(&world, "overworld", 5, 5).unwrap_err();
        assert!(err.contains("尚未生成"), "实际报错: {err}");
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn dimension_dirs_differ() {
        let world = Path::new("/tmp/w");
        assert!(region_dir(world, "overworld").ends_with("region"));
        assert!(region_dir(world, "nether").ends_with("DIM-1/region"));
        assert!(region_dir(world, "end").ends_with("DIM1/region"));
        assert_eq!(entry_index(0, 0), 0);
        // 33 → 相对 1；-1 → 相对 31
        assert_eq!(entry_index(33, -1), 1 + 31 * 32);
        assert_eq!(entry_index(-1, -1), 31 + 31 * 32);
    }

    #[test]
    fn concurrent_updates_to_one_region_keep_both_changes() {
        let world = tempfile::tempdir().unwrap();
        let root = world.path();
        write_chunk(root, "overworld", 0, 0, &chunk_with("before")).unwrap();
        write_chunk(root, "overworld", 1, 0, &chunk_with("before")).unwrap();
        std::thread::scope(|threads| {
            for cx in 0..=1 {
                threads.spawn(move || {
                    edit_chunk(root, "overworld", cx, 0, |chunk| {
                        *chunk = chunk_with(&format!("after {cx}"));
                        Ok(())
                    }).unwrap();
                });
            }
        });
        for cx in 0..=1 {
            let chunk = read_chunk(root, "overworld", cx, 0).unwrap();
            assert_eq!(level::get_string(&chunk, "Note"), Some(format!("after {cx}")));
        }
    }

    #[test]
    fn unreadable_existing_header_is_never_treated_as_a_missing_region() {
        let world = tempfile::tempdir().unwrap();
        let path = region_file(world.path(), "overworld", 0, 0);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"truncated").unwrap();
        assert!(write_chunk(world.path(), "overworld", 0, 0, &chunk_with("replacement")).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"truncated");
    }
}
