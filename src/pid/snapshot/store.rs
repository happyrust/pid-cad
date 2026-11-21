//! 快照的存取（SPEC-pid-snapshot §3.1、§3.3、§3.4）。每张图的快照放在
//! `<图纸目录>/pid-snapshots/<图号>/`：快照文件名是四位序号（`0001.json`，超过 9999 自然
//! 加位），旁边一份目录清单 `index.json`。
//!
//! 有哪些快照以目录里的文件为准。清单只是为了列表快、记住用过的最大序号：清单丢了、坏了，
//! 或者目录里有清单没记的文件，就读文件补齐重写；清单里有而文件不在的，列表里标
//! [`SnapshotStatus::Missing`]。快照文件只新建、不覆盖，写完设成只读。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{check_fields, parse, Snapshot, SnapshotError, UTF8_BOM};

/// 图纸目录下放快照的目录名。
pub const SNAPSHOTS_DIR: &str = "pid-snapshots";
/// 目录清单的文件名。
pub const INDEX_FILE: &str = "index.json";
/// 目录清单格式的版本。
const INDEX_VERSION: u32 = 1;
/// 要用的序号已经有文件时换下一个，连第一次在内最多试这么多次（§3.4 第 2 条）。
const ATTEMPTS: usize = 3;

/// 写好的一份快照（§5.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    /// 序号。直接写到指定文件的是快照里原有的序号，`capture` 出来的是 `None`。
    pub seq: Option<u64>,
    pub path: PathBuf,
    /// 文件的字节数。
    pub bytes: u64,
    /// 整个文件的 SHA-256，小写十六进制。
    pub sha256: String,
}

/// 列表里的一份：目录清单里记的各项（§3.3），加上文件还在不在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotInfo {
    pub seq: u64,
    /// 文件名，如 `0001.json`。
    pub file: String,
    pub created_at: u64,
    pub created_by: String,
    pub reason: String,
    /// 对象数（快照的 `counts.objects`）。
    pub objects: usize,
    /// 文件的字节数。
    pub bytes: u64,
    /// 整个文件的 SHA-256。
    pub sha256: String,
    pub status: SnapshotStatus,
}

/// 清单里记的快照文件还在不在（§5.3 的 `status`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SnapshotStatus {
    #[serde(rename = "ok")]
    Present,
    #[serde(rename = "missing")]
    Missing,
}

/// 目录清单（§3.3）。只是缓存，有哪些快照以目录里的文件为准。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Index {
    version: u32,
    drawing_id: String,
    /// 用过的最大序号，只增不减。
    last_seq: u64,
    /// 按序号从小到大。
    snapshots: Vec<Entry>,
}

impl Index {
    /// 认得：这一版的格式，序号从 1 起、从小到大不重复，文件名就是序号的写法。
    fn is_sound(&self) -> bool {
        self.version == INDEX_VERSION
            && self
                .snapshots
                .windows(2)
                .all(|pair| pair[0].seq < pair[1].seq)
            && self
                .snapshots
                .iter()
                .all(|entry| entry.seq > 0 && entry.file == file_name(entry.seq))
    }
}

/// 清单里的一份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    seq: u64,
    file: String,
    created_at: u64,
    created_by: String,
    reason: String,
    objects: usize,
    bytes: u64,
    sha256: String,
}

impl Entry {
    /// 第 `seq` 份快照、文件内容是 `bytes` 时清单里记的一份。
    fn of(snapshot: &Snapshot, seq: u64, bytes: &[u8]) -> Self {
        Self {
            seq,
            file: file_name(seq),
            created_at: snapshot.created_at,
            created_by: snapshot.created_by.clone(),
            reason: snapshot.reason.clone(),
            objects: snapshot.counts.objects,
            bytes: bytes.len() as u64,
            sha256: sha256(bytes),
        }
    }

    fn info(self, present: bool) -> SnapshotInfo {
        SnapshotInfo {
            seq: self.seq,
            file: self.file,
            created_at: self.created_at,
            created_by: self.created_by,
            reason: self.reason,
            objects: self.objects,
            bytes: self.bytes,
            sha256: self.sha256,
            status: if present {
                SnapshotStatus::Present
            } else {
                SnapshotStatus::Missing
            },
        }
    }
}

/// 某张图的快照目录：`<图纸目录>/pid-snapshots/<图号>`。图号里 Windows 文件名不允许的字符
/// （`< > : " / \ | ? *` 和控制字符）、首尾的空格和句点换成 `_`；空的图号写成 `_`，设备名
/// （`CON`、`NUL`、`COM1` 之类，不分大小写）前面加 `_`。
#[must_use]
pub fn dir_for(drawing_dir: &Path, drawing_id: &str) -> PathBuf {
    drawing_dir.join(SNAPSHOTS_DIR).join(dir_name(drawing_id))
}

fn dir_name(drawing_id: &str) -> String {
    let mut name: Vec<char> = drawing_id
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let edge = |c: &&char| matches!(**c, ' ' | '.');
    let lead = name.iter().take_while(edge).count();
    let trail = name.iter().rev().take_while(edge).count();
    let end = name.len();
    name[..lead].fill('_');
    name[end - trail..].fill('_');
    let name: String = name.into_iter().collect();
    if name.is_empty() {
        return "_".to_owned();
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.len() == 4 && stem.starts_with(prefix) && stem.as_bytes()[3].is_ascii_digit()
    };
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT")
    {
        format!("_{name}")
    } else {
        name
    }
}

/// 分配序号并写入（§3.4），返回序号、文件路径、字节数、校验和。新序号 = max(清单的
/// `last_seq`，清单和目录里最大的序号) + 1；那个文件已经有了（别的程序刚用了这个序号）就换
/// 下一个，连第一次在内试三次仍不行报 [`SNAPSHOT_IMMUTABLE`](super::SNAPSHOT_IMMUTABLE)。存过的快照
/// （带序号的）不能再存，也报这个码。写完设成只读，再更新清单。
pub fn save(dir: &Path, snapshot: Snapshot) -> Result<Saved, SnapshotError> {
    save_with(dir, snapshot, |_| {})
}

/// [`save`] 的本体。`before_create` 在每次新建快照文件之前调用，测试用它模拟别的程序抢先
/// 用掉序号。
fn save_with(
    dir: &Path,
    mut snapshot: Snapshot,
    mut before_create: impl FnMut(&Path),
) -> Result<Saved, SnapshotError> {
    if let Some(seq) = snapshot.seq {
        return Err(SnapshotError::Immutable(format!(
            "这份快照已经是第 {seq} 份：序号只在第一次存的时候分配，存过的快照不能再存"
        )));
    }
    check_fields(&snapshot)?;
    fs::create_dir_all(dir).map_err(|error| io_error(dir, &error))?;
    let mut taken = Vec::new();
    let mut floor = 1;
    for _ in 0..ATTEMPTS {
        // 至少比上一次试的大一：占着那个序号的若不是认得出的快照文件（同名的目录，Windows 上
        // 大小写不同的 `0002.JSON`），目录里数不到它，只按目录算会一直试同一个序号。
        let seq = next_seq(dir)?.max(floor);
        floor = seq.saturating_add(1);
        snapshot.seq = Some(seq);
        let bytes = snapshot.to_bytes();
        let path = dir.join(file_name(seq));
        before_create(&path);
        match write_new(&path, &bytes) {
            Ok(()) => {
                let entry = Entry::of(&snapshot, seq, &bytes);
                let saved = Saved {
                    seq: Some(seq),
                    path,
                    bytes: entry.bytes,
                    sha256: entry.sha256.clone(),
                };
                // 清单更新不了不算失败：快照已经写好，下次列表时照文件补上。
                let _ = sync(dir, Some((entry, snapshot.drawing_id.as_str())));
                return Ok(saved);
            }
            Err(SnapshotError::Immutable(_)) => taken.push(seq.to_string()),
            Err(error) => return Err(error),
        }
    }
    Err(SnapshotError::Immutable(format!(
        "序号 {} 的文件都已经有了，像是别的程序也在往 {} 存快照",
        taken.join("、"),
        dir.display()
    )))
}

/// 不分配序号，直接写到 `path`（§5.2 的 `FILE` 形式），序号照快照里的写（`capture` 出来的
/// 是 `null`）。文件已存在报 [`SNAPSHOT_IMMUTABLE`](super::SNAPSHOT_IMMUTABLE)，不覆盖；写完设成只读。
pub fn save_to_file(path: &Path, snapshot: &Snapshot) -> Result<Saved, SnapshotError> {
    check_fields(snapshot)?;
    let bytes = snapshot.to_bytes();
    write_new(path, &bytes)?;
    Ok(Saved {
        seq: snapshot.seq,
        path: path.to_owned(),
        bytes: bytes.len() as u64,
        sha256: sha256(&bytes),
    })
}

/// 列出一张图的快照，按序号从小到大。先照目录里的文件整理清单（见模块说明）；目录不存在时
/// 是空的，也不建目录。
pub fn list(dir: &Path) -> Result<Vec<SnapshotInfo>, SnapshotError> {
    let (index, files) = sync(dir, None)?;
    Ok(index
        .snapshots
        .into_iter()
        .map(|entry| {
            let present = files.contains(&entry.seq);
            entry.info(present)
        })
        .collect())
}

/// 按序号读一份快照并校验：文件不在报 [`SNAPSHOT_NOT_FOUND`](super::SNAPSHOT_NOT_FOUND)；与清单里记的
/// 校验和不同、文件里记的序号不是 `seq`，都报 [`SNAPSHOT_CORRUPT`](super::SNAPSHOT_CORRUPT)；其余同
/// [`parse`]。
pub fn load(dir: &Path, seq: u64) -> Result<Snapshot, SnapshotError> {
    read_numbered(dir, seq, recorded(dir, seq).as_deref()).map(|(snapshot, _)| snapshot)
}

/// 读一个快照文件并校验，同 [`parse`]；文件不在报 [`SNAPSHOT_NOT_FOUND`](super::SNAPSHOT_NOT_FOUND)。
pub fn load_file(path: &Path) -> Result<Snapshot, SnapshotError> {
    parse(&read(path)?)
}

/// 把第 `seq` 份原样复制到 `to`，返回字节数。先照 [`load`] 校验，坏了的不导出；`to` 已经存在报
/// [`SNAPSHOT_IMMUTABLE`](super::SNAPSHOT_IMMUTABLE)，不覆盖。导出的文件也设成只读。
pub fn export(dir: &Path, seq: u64, to: &Path) -> Result<u64, SnapshotError> {
    let (_, bytes) = read_numbered(dir, seq, recorded(dir, seq).as_deref())?;
    write_new(to, &bytes)?;
    Ok(bytes.len() as u64)
}

/// 下一个序号：清单记的 `last_seq`、清单里最大的序号、目录里文件名最大的序号，三者取大加一。
fn next_seq(dir: &Path) -> Result<u64, SnapshotError> {
    let (index, last_seq) = read_index(dir)?;
    let listed = index
        .and_then(|index| index.snapshots.last().map(|entry| entry.seq))
        .unwrap_or(0);
    let named = files(dir)?.last().copied().unwrap_or(0);
    Ok(last_seq.max(listed).max(named).saturating_add(1))
}

/// 照目录里的文件整理清单，返回整理好的清单和目录里有文件的序号。清单没记的文件读出来补上，
/// 读不出来的不列（它的序号照样算用过）；清单里有而文件不在的留着。`fresh` 是刚写好的一份和
/// 它的图号，免得再读一遍。清单变了就重写。
fn sync(dir: &Path, fresh: Option<(Entry, &str)>) -> Result<(Index, BTreeSet<u64>), SnapshotError> {
    let files = files(dir)?;
    let (stored, last_seq) = read_index(dir)?;
    let rebuild = stored.is_none();
    let mut index = stored.unwrap_or(Index {
        version: INDEX_VERSION,
        drawing_id: String::new(),
        last_seq,
        snapshots: Vec::new(),
    });
    let mut entries: BTreeMap<u64, Entry> = index
        .snapshots
        .drain(..)
        .map(|entry| (entry.seq, entry))
        .collect();
    let mut changed = rebuild;
    let fresh_id = fresh.map(|(entry, drawing_id)| {
        entries.insert(entry.seq, entry);
        drawing_id
    });
    for &seq in &files {
        if entries.contains_key(&seq) {
            continue;
        }
        if let Ok((snapshot, bytes)) = read_numbered(dir, seq, None) {
            if rebuild || index.drawing_id.is_empty() {
                index.drawing_id.clone_from(&snapshot.drawing_id);
            }
            entries.insert(seq, Entry::of(&snapshot, seq, &bytes));
            changed = true;
        }
    }
    if let Some(drawing_id) = fresh_id {
        drawing_id.clone_into(&mut index.drawing_id);
        changed = true;
    }
    let newest = files
        .last()
        .into_iter()
        .chain(entries.keys())
        .copied()
        .fold(index.last_seq, u64::max);
    if newest != index.last_seq {
        index.last_seq = newest;
        changed = true;
    }
    index.snapshots = entries.into_values().collect();
    if changed && index.last_seq > 0 {
        // 清单只是缓存：写不进去（目录只读之类）不算错，下次再整理。
        let _ = write_index(dir, &index);
    }
    Ok((index, files))
}

/// 读清单。认得的是 `Some`；没有或者坏了（不是 JSON、版本不对、字段不对、序号没排好或重了、
/// 文件名与序号对不上）是 `None`。第二项是清单里认得出的 `last_seq`：坏了的清单也尽量留下它，
/// 序号不往回退。
fn read_index(dir: &Path) -> Result<(Option<Index>, u64), SnapshotError> {
    let path = dir.join(INDEX_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((None, 0)),
        Err(error) => return Err(io_error(&path, &error)),
    };
    let text = bytes.strip_prefix(UTF8_BOM).unwrap_or(&bytes);
    let Ok(value) = serde_json::from_slice::<Value>(text) else {
        return Ok((None, 0));
    };
    let last_seq = value.get("last_seq").and_then(Value::as_u64).unwrap_or(0);
    let index = serde_json::from_value::<Index>(value)
        .ok()
        .filter(Index::is_sound);
    Ok((index, last_seq))
}

/// 先写到旁边的临时文件再改名盖过去：失败时原来的清单不变，临时文件删掉。
fn write_index(dir: &Path, index: &Index) -> io::Result<()> {
    let path = dir.join(INDEX_FILE);
    let temporary = dir.join(format!("{INDEX_FILE}.{}.tmp", std::process::id()));
    let text = serde_json::to_vec(index).expect("清单能写成 JSON");
    let result = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// 目录里快照文件的序号。只认 `0001.json` 这样的名字：四位起、不多补零、小写的 `.json`。
/// 目录不存在算没有。
fn files(dir: &Path) -> Result<BTreeSet<u64>, SnapshotError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => return Err(io_error(dir, &error)),
    };
    let mut seqs = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| io_error(dir, &error))?;
        let Some(seq) = entry.file_name().to_str().and_then(seq_of) else {
            continue;
        };
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            seqs.insert(seq);
        }
    }
    Ok(seqs)
}

fn file_name(seq: u64) -> String {
    format!("{seq:04}.json")
}

/// 文件名是第几份快照的；不是快照文件的名字是 `None`。
fn seq_of(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(".json")?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let seq = digits.parse().ok()?;
    (seq > 0 && file_name(seq) == name).then_some(seq)
}

/// 清单里记的第 `seq` 份的校验和。清单没有、坏了、读不了、没记这一份都是 `None`。
fn recorded(dir: &Path, seq: u64) -> Option<String> {
    let (index, _) = read_index(dir).ok()?;
    index?
        .snapshots
        .into_iter()
        .find(|entry| entry.seq == seq)
        .map(|entry| entry.sha256)
}

/// 读第 `seq` 份并校验：文件在；给了 `recorded`（清单里记的校验和）时与整个文件的相同；
/// 内容是一份好快照，里面记的序号就是 `seq`。返回快照和文件的字节。
fn read_numbered(
    dir: &Path,
    seq: u64,
    recorded: Option<&str>,
) -> Result<(Snapshot, Vec<u8>), SnapshotError> {
    let path = dir.join(file_name(seq));
    let bytes = read(&path)?;
    if let Some(recorded) = recorded {
        let actual = sha256(&bytes);
        if !actual.eq_ignore_ascii_case(recorded) {
            return Err(SnapshotError::Corrupt(format!(
                "{} 的校验和是 {actual}，目录清单里记的是 {recorded}",
                path.display()
            )));
        }
    }
    let snapshot = parse(&bytes)?;
    if snapshot.seq != Some(seq) {
        let inside = match snapshot.seq {
            Some(inside) => inside.to_string(),
            None => "空的".to_owned(),
        };
        return Err(SnapshotError::Corrupt(format!(
            "{} 里记的序号是 {inside}，与文件名对不上",
            path.display()
        )));
    }
    Ok((snapshot, bytes))
}

/// 新建 `path` 写进 `bytes`，写完设成只读。已经存在（文件、目录都算）报
/// [`SnapshotError::Immutable`]，不动它；写到一半出错就删掉写了一半的文件。
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        // Windows 上同名的是目录时报的是「拒绝访问」，不是「已存在」。
        Err(error)
            if error.kind() == io::ErrorKind::AlreadyExists
                || fs::symlink_metadata(path).is_ok() =>
        {
            return Err(SnapshotError::Immutable(format!(
                "{} 已经存在，快照不覆盖已有的文件",
                path.display()
            )));
        }
        Err(error) => return Err(io_error(path, &error)),
    };
    let written = file.write_all(bytes);
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(path);
        return Err(io_error(path, &error));
    }
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        // 设不成只读不算失败：快照已经写好，报错的话调用方会当没存上、再存一份。
        let _ = fs::set_permissions(path, permissions);
    }
    Ok(())
}

/// 读整个文件；不存在报 [`SnapshotError::NotFound`]。
fn read(path: &Path) -> Result<Vec<u8>, SnapshotError> {
    fs::read(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            SnapshotError::NotFound(format!("{} 不存在", path.display()))
        } else {
            io_error(path, &error)
        }
    })
}

/// 整个文件的 SHA-256，小写十六进制。
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn io_error(path: &Path, error: &io::Error) -> SnapshotError {
    SnapshotError::Io(format!("{}：{error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::pid::snapshot::tests::{drawing, meta};
    use crate::pid::snapshot::{
        capture, parse, SNAPSHOT_CORRUPT, SNAPSHOT_IMMUTABLE, SNAPSHOT_NOT_FOUND,
    };
    use SnapshotStatus::{Missing, Present};

    /// 测试用的临时目录，测完删掉（只读的快照文件也删得掉）。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            // 测试并行跑：计数器把同一纳秒里建的两个目录分开。
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "ocs-snapshot-store-{}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            fs::create_dir(&path).expect("scratch directory");
            Self(path)
        }

        /// 这个临时目录里 PID-1101 的快照目录。
        fn snapshots(&self) -> PathBuf {
            dir_for(&self.0, "PID-1101")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 20 个对象的合成图做的一份快照，还没存。
    fn snapshot(reason: &str) -> Snapshot {
        let (summary, graph) = drawing(20);
        capture(summary, graph, meta(reason)).expect("能做快照")
    }

    /// 同一份快照存成第 `seq` 份以后的样子。
    fn numbered(reason: &str, seq: u64) -> Snapshot {
        let mut snapshot = snapshot(reason);
        snapshot.seq = Some(seq);
        snapshot
    }

    /// 列表里各份的序号和状态。
    fn listed(dir: &Path) -> Vec<(u64, SnapshotStatus)> {
        list(dir)
            .expect("能列")
            .into_iter()
            .map(|info| (info.seq, info.status))
            .collect()
    }

    /// 目录里的文件名，排好序。
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("目录在")
            .map(|entry| {
                entry
                    .expect("目录项")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    fn read(path: &Path) -> Vec<u8> {
        fs::read(path).expect("文件在")
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn readonly(path: &Path) -> bool {
        fs::metadata(path).expect("文件在").permissions().readonly()
    }

    /// 把文件换成 `bytes`：快照文件是只读的，先删掉再写。
    fn rewrite(path: &Path, bytes: &[u8]) {
        fs::remove_file(path).expect("只读的文件也删得掉");
        fs::write(path, bytes).expect("能写");
    }

    /// 连存三次，序号是 1、2、3；删掉 `0003.json` 再存，序号是 4，3 不再用、列表里标
    /// `missing`（§6 第 1 条）。还没有快照时列表是空的，不建目录。清单的字段和顺序照 §3.3。
    #[test]
    fn numbers_go_up_and_are_never_reused() {
        let scratch = Scratch::new();
        let dir = scratch.snapshots();
        assert_eq!(dir, scratch.0.join("pid-snapshots").join("PID-1101"));
        assert!(list(&dir).expect("能列").is_empty());
        assert!(!dir.exists(), "列表不建目录");

        for (seq, reason) in (1..).zip(["第一份", "第二份", "第三份"]) {
            let saved = save(&dir, snapshot(reason)).expect("能存");
            let path = dir.join(format!("{seq:04}.json"));
            let bytes = read(&path);
            assert_eq!(
                saved,
                Saved {
                    seq: Some(seq),
                    path,
                    bytes: bytes.len() as u64,
                    sha256: sha256(&bytes),
                }
            );
            assert_eq!(parse(&bytes).expect("读得回来"), numbered(reason, seq));
        }
        let infos = list(&dir).expect("能列");
        let second = read(&dir.join("0002.json"));
        assert_eq!(infos.len(), 3);
        assert_eq!(
            infos[1],
            SnapshotInfo {
                seq: 2,
                file: "0002.json".to_owned(),
                created_at: 1_794_800_000,
                created_by: "zhangsan".to_owned(),
                reason: "第二份".to_owned(),
                objects: snapshot("第二份").counts.objects,
                bytes: second.len() as u64,
                sha256: sha256(&second),
                status: Present,
            }
        );
        let index = String::from_utf8(read(&dir.join(INDEX_FILE))).expect("UTF-8");
        let first = format!(
            "{{\"version\":1,\"drawing_id\":\"PID-1101\",\"last_seq\":3,\"snapshots\":[{{\"seq\":1,\
             \"file\":\"0001.json\",\"created_at\":1794800000,\"created_by\":\"zhangsan\",\
             \"reason\":\"第一份\",\"objects\":{},\"bytes\":{},\"sha256\":\"{}\"}},",
            infos[0].objects, infos[0].bytes, infos[0].sha256
        );
        assert!(index.starts_with(&first), "{index}");

        fs::remove_file(dir.join("0003.json")).expect("只读的文件也删得掉");
        assert_eq!(listed(&dir), [(1, Present), (2, Present), (3, Missing)]);
        assert_eq!(save(&dir, snapshot("第四份")).expect("能存").seq, Some(4));
        assert_eq!(
            listed(&dir),
            [(1, Present), (2, Present), (3, Missing), (4, Present)]
        );
        assert_eq!(
            names(&dir),
            ["0001.json", "0002.json", "0004.json", INDEX_FILE]
        );
    }

    /// 要用的序号刚被别的程序写了文件：换下一个序号，别人的文件不动；连着三次都被抢先，
    /// 报 `SNAPSHOT_IMMUTABLE`（§3.4 第 2 条）。占着序号的是目录里数不到的东西，也换下一个。
    #[test]
    fn a_number_taken_meanwhile_is_skipped_at_most_three_times() {
        let scratch = Scratch::new();
        let dir = scratch.snapshots();
        save(&dir, snapshot("第一份")).expect("能存");

        let mut taken = Vec::new();
        let saved = save_with(&dir, snapshot("第二份"), |path| {
            if taken.is_empty() {
                fs::write(path, "别的程序写的").expect("抢先写");
                taken.push(path.to_owned());
            }
        })
        .expect("换个序号存");
        assert_eq!(taken, [dir.join("0002.json")]);
        assert_eq!(saved.seq, Some(3));
        assert_eq!(read(&taken[0]), "别的程序写的".as_bytes());
        assert_eq!(
            parse(&read(&saved.path)).expect("读得回来"),
            numbered("第二份", 3)
        );

        let mut tried = Vec::new();
        let error = save_with(&dir, snapshot("第三份"), |path| {
            fs::write(path, "别的程序写的").expect("抢先写");
            tried.push(
                path.file_name()
                    .expect("文件名")
                    .to_string_lossy()
                    .into_owned(),
            );
        })
        .expect_err("连着三次被抢先");
        assert_eq!(error.code(), SNAPSHOT_IMMUTABLE);
        assert_eq!(tried, ["0004.json", "0005.json", "0006.json"]);
        for name in &tried {
            assert_eq!(read(&dir.join(name)), "别的程序写的".as_bytes());
        }
        assert_eq!(
            names(&dir),
            [
                "0001.json",
                "0002.json",
                "0003.json",
                "0004.json",
                "0005.json",
                "0006.json",
                INDEX_FILE
            ]
        );

        // 占着序号的若不是认得出的快照文件（这里是同名的目录；Windows 上大小写不同的
        // `0007.JSON` 也一样），目录里数不到它，照样换下一个序号，不再试同一个。
        let mut blocked = Vec::new();
        let saved = save_with(&dir, snapshot("第四份"), |path| {
            if blocked.is_empty() {
                fs::create_dir(path).expect("抢先建同名目录");
                blocked.push(path.to_owned());
            }
        })
        .expect("换个序号存");
        assert_eq!(blocked, [dir.join("0007.json")]);
        assert_eq!(saved.seq, Some(8));
        assert!(blocked[0].is_dir(), "别人的目录不动");
    }

    /// 存下的快照文件是只读的。往已有的序号写（直接写到那个文件、把读回来的快照再存一次），
    /// 直接写文件、导出时目标已存在（是目录也算），都报 `SNAPSHOT_IMMUTABLE`，已有的文件一个字节也不变。
    #[test]
    fn no_way_overwrites_an_existing_file() {
        let scratch = Scratch::new();
        let dir = scratch.snapshots();
        let first = save(&dir, snapshot("第一份")).expect("能存");
        assert!(readonly(&first.path), "存下的快照是只读的");
        let original = read(&first.path);

        let error = save_to_file(&first.path, &snapshot("想盖掉第一份")).expect_err("不覆盖");
        assert_eq!(error.code(), SNAPSHOT_IMMUTABLE);
        let loaded = load(&dir, 1).expect("能读");
        let error = save(&dir, loaded).expect_err("带序号的快照不能再存");
        assert_eq!(error.code(), SNAPSHOT_IMMUTABLE);

        let file = scratch.0.join("送审.json");
        let written = save_to_file(&file, &snapshot("送审")).expect("能写");
        let bytes = read(&file);
        assert_eq!(
            written,
            Saved {
                seq: None,
                path: file.clone(),
                bytes: bytes.len() as u64,
                sha256: sha256(&bytes),
            }
        );
        assert!(readonly(&file), "直接写的快照也是只读的");
        assert_eq!(load_file(&file).expect("读得回来"), snapshot("送审"));
        let error = save_to_file(&file, &snapshot("再写一次")).expect_err("已存在");
        assert_eq!(error.code(), SNAPSHOT_IMMUTABLE);
        let error = export(&dir, 1, &file).expect_err("导出也不覆盖");
        assert_eq!(error.code(), SNAPSHOT_IMMUTABLE);
        let error = save_to_file(&scratch.0, &snapshot("写到目录上")).expect_err("目录也不盖");
        assert_eq!(error.code(), SNAPSHOT_IMMUTABLE);

        assert_eq!(read(&file), bytes);
        assert_eq!(read(&first.path), original);
        assert_eq!(names(&dir), ["0001.json", INDEX_FILE]);
    }

    /// 清单丢了、坏了、少记了一份：列表照目录里的文件补齐，重写出来与原来的清单逐字节相同。
    /// 清单多记了（文件不在）的标 `missing`；`last_seq` 只增不减，坏了的清单能认出它也留下。
    /// 目录里不像快照的文件不算；名字像、读不出来的不列，它的序号照样算用过。
    #[test]
    fn the_index_is_rebuilt_from_the_files() {
        let scratch = Scratch::new();
        let dir = scratch.snapshots();
        for reason in ["第一份", "第二份", "第三份"] {
            save(&dir, snapshot(reason)).expect("能存");
        }
        let index_path = dir.join(INDEX_FILE);
        let infos = list(&dir).expect("能列");
        let index = read(&index_path);

        let mut one_short: Value = serde_json::from_slice(&index).expect("清单是 JSON");
        one_short["snapshots"]
            .as_array_mut()
            .expect("数组")
            .remove(1);
        let broken = [
            None,
            Some(b"{\"version\":1,\"snap".to_vec()),
            Some(one_short.to_string().into_bytes()),
        ];
        for broken in broken {
            match broken {
                None => fs::remove_file(&index_path).expect("能删"),
                Some(bytes) => fs::write(&index_path, bytes).expect("能写"),
            }
            assert_eq!(list(&dir).expect("能列"), infos);
            assert_eq!(read(&index_path), index, "重写出来的清单与原来的相同");
        }

        let mut one_more: Value = serde_json::from_slice(&index).expect("清单是 JSON");
        let mut seventh = one_more["snapshots"][2].clone();
        seventh["seq"] = json!(7);
        seventh["file"] = json!("0007.json");
        one_more["snapshots"]
            .as_array_mut()
            .expect("数组")
            .push(seventh);
        one_more["last_seq"] = json!(7);
        fs::write(&index_path, one_more.to_string()).expect("能写");
        assert_eq!(
            listed(&dir),
            [(1, Present), (2, Present), (3, Present), (7, Missing)]
        );
        assert_eq!(save(&dir, snapshot("第八份")).expect("能存").seq, Some(8));

        fs::write(&index_path, r#"{"version":2,"last_seq":20}"#).expect("能写");
        assert_eq!(
            listed(&dir),
            [(1, Present), (2, Present), (3, Present), (8, Present)]
        );
        assert_eq!(
            save(&dir, snapshot("第二十一份")).expect("能存").seq,
            Some(21)
        );

        fs::remove_file(&index_path).expect("能删");
        fs::write(dir.join("0030.json"), "不是快照").expect("能写");
        for name in [
            "31.json",
            "00032.json",
            "0033.json.bak",
            "0034.JSON",
            "说明.txt",
        ] {
            fs::write(dir.join(name), "{}").expect("能写");
        }
        assert_eq!(
            listed(&dir),
            [
                (1, Present),
                (2, Present),
                (3, Present),
                (8, Present),
                (21, Present)
            ]
        );
        assert_eq!(save(&dir, snapshot("下一份")).expect("能存").seq, Some(31));
    }

    /// 文件与清单里记的校验和不同报 `SNAPSHOT_CORRUPT`（哪怕单看文件是一份好快照），列表不拿它
    /// 重建清单，导出也不导；文件里记的序号与文件名对不上也报 `SNAPSHOT_CORRUPT`。
    #[test]
    fn a_file_unlike_the_index_is_corrupt() {
        let scratch = Scratch::new();
        let dir = scratch.snapshots();
        for reason in ["第一份", "第二份"] {
            save(&dir, snapshot(reason)).expect("能存");
        }
        let second = dir.join("0002.json");
        let value: Value = serde_json::from_slice(&read(&second)).expect("JSON");
        rewrite(&second, &serde_json::to_vec_pretty(&value).expect("能写"));
        assert!(
            load_file(&second).is_ok(),
            "缩进了、值没变，单看文件仍是好快照"
        );

        let index = read(&dir.join(INDEX_FILE));
        let error = load(&dir, 2).expect_err("与清单不符");
        assert_eq!(error.code(), SNAPSHOT_CORRUPT);
        assert!(error.to_string().contains("目录清单"), "{error}");
        assert_eq!(listed(&dir), [(1, Present), (2, Present)]);
        assert_eq!(read(&dir.join(INDEX_FILE)), index, "不按改过的文件重建清单");
        assert_eq!(load(&dir, 2).expect_err("仍不符").code(), SNAPSHOT_CORRUPT);
        let to = scratch.0.join("第二份.json");
        assert_eq!(
            export(&dir, 2, &to).expect_err("坏的不导出").code(),
            SNAPSHOT_CORRUPT
        );
        assert!(!to.exists());

        fs::remove_file(dir.join(INDEX_FILE)).expect("能删");
        rewrite(&second, &read(&dir.join("0001.json")));
        let error = load(&dir, 2).expect_err("文件里是第一份");
        assert_eq!(error.code(), SNAPSHOT_CORRUPT);
        assert!(error.to_string().contains("序号"), "{error}");
        assert_eq!(listed(&dir), [(1, Present)]);
    }

    /// 导出序号 2 得到的文件与 `0002.json` 逐字节相同，也是只读的（§6 第 3 条）。按序号读、
    /// 读导出的文件，都是存进去的那一份。没有的序号、没有的文件报 `SNAPSHOT_NOT_FOUND`。
    #[test]
    fn export_copies_byte_for_byte() {
        let scratch = Scratch::new();
        let dir = scratch.snapshots();
        for reason in ["第一份", "第二份", "第三份"] {
            save(&dir, snapshot(reason)).expect("能存");
        }
        let to = scratch.0.join("第二份.json");
        let bytes = export(&dir, 2, &to).expect("能导出");
        assert_eq!(read(&to), read(&dir.join("0002.json")));
        assert_eq!(bytes, read(&to).len() as u64);
        assert!(readonly(&to), "导出的文件也是只读的");
        assert_eq!(load(&dir, 2).expect("能读"), numbered("第二份", 2));
        assert_eq!(load_file(&to).expect("能读"), numbered("第二份", 2));

        let nowhere = scratch.0.join("第九份.json");
        assert_eq!(
            export(&dir, 9, &nowhere).expect_err("没有第九份").code(),
            SNAPSHOT_NOT_FOUND
        );
        assert!(!nowhere.exists());
        assert_eq!(
            load(&dir, 9).expect_err("没有第九份").code(),
            SNAPSHOT_NOT_FOUND
        );
        assert_eq!(
            load_file(&nowhere).expect_err("没有这个文件").code(),
            SNAPSHOT_NOT_FOUND
        );
    }

    /// 图号里 Windows 文件名不允许的字符、首尾的空格和句点换成 `_`（§3.1）；控制字符也换，
    /// 空图号写成 `_`，设备名前面加 `_`：换出来的都建得成目录。
    #[test]
    fn the_drawing_number_becomes_a_directory_name() {
        let base = Path::new("工程");
        let name = |drawing_id: &str| {
            let dir = dir_for(base, drawing_id);
            assert_eq!(dir.parent(), Some(base.join(SNAPSHOTS_DIR).as_path()));
            dir.file_name()
                .expect("目录名")
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(name("PID-1101"), "PID-1101");
        assert_eq!(name("制氢 1101"), "制氢 1101");
        assert_eq!(name(r#"A<B>C:D"E/F\G|H?I*J"#), "A_B_C_D_E_F_G_H_I_J");
        assert_eq!(name(" .PID 1101. "), "__PID 1101__");
        assert_eq!(name("PID\t1101\n"), "PID_1101_");
        assert_eq!(name(""), "_");
        assert_eq!(name(".."), "__");
        assert_eq!(name("con"), "_con");
        assert_eq!(name("COM1.dwg"), "_COM1.dwg");
        assert_eq!(name("CONSOLE"), "CONSOLE");

        let scratch = Scratch::new();
        for drawing_id in [" A<B>:C. ", "nul", "lpt9", ""] {
            fs::create_dir_all(dir_for(&scratch.0, drawing_id)).expect("建得出目录");
        }
        assert_eq!(
            names(&scratch.0.join(SNAPSHOTS_DIR)),
            ["_", "_A_B__C__", "_lpt9", "_nul"]
        );
    }
}
