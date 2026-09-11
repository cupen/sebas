//! 节点本地会话日志（add-remote-execution-node 4.1 / 4.4）。
//!
//! 契约（[`node-session-channel`] spec）：
//!
//! - **有序**：会话内 `seq` 单调递增；节点是唯一写者，因此不存在并发写入冲突。
//! - **纪元**：`epoch` 只在日志被重置/丢失时递增，控制面据此把时间线标记为不连续，
//!   而不是把两条时间线接成一条。
//! - **保真**：传输层可以合并（4.2），但这里的原始序列永远完整——控制面按 seq
//!   回拉即得精确内容。
//! - **回收留下痕迹**：回收必须推进 `reclaimed_through` 水位线，控制面据此把缺口
//!   标成「节点已回收」而不是永远 pending。
//!
//! 落盘形态：每个会话一个目录下的两个文件——`<id>.log.jsonl`（一行一条，只追加）
//! 与 `<id>.meta.json`（纪元与水位线，原子重写）。只追加让正常路径没有重写风险；
//! 回收与重置才重写，且都走「临时文件 + rename」。

use sebas_node_link::LogEntry;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// 日志错误。
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// 磁盘读写失败。
    #[error("会话日志不可用：{cause}")]
    Io {
        /// 成因（含路径）。
        cause: String,
    },
}

impl LogError {
    fn io(cause: impl Into<String>) -> Self {
        Self::Io {
            cause: cause.into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Meta {
    epoch: u64,
    reclaimed_through: u64,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            epoch: 1,
            reclaimed_through: 0,
        }
    }
}

/// 单个会话的日志。
#[derive(Debug)]
pub struct SessionLog {
    dir: PathBuf,
    session_id: String,
    meta: Meta,
    entries: VecDeque<LogEntry>,
    next_seq: u64,
}

impl SessionLog {
    /// 打开（或建立）某会话的日志。已存在则读回：**重启后序列继续**。
    pub fn open(dir: impl Into<PathBuf>, session_id: &str) -> Result<Self, LogError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| LogError::io(format!("无法创建 {}：{e}", dir.display())))?;
        let meta = read_meta(&meta_path(&dir, session_id))?;
        let entries = read_entries(&log_path(&dir, session_id))?;
        let next_seq = entries.back().map(|e| e.seq + 1).unwrap_or(1).max(1);
        Ok(Self {
            dir,
            session_id: session_id.to_string(),
            meta,
            entries,
            next_seq,
        })
    }

    /// 日志纪元。
    pub fn epoch(&self) -> u64 {
        self.meta.epoch
    }

    /// 已回收到（含）的 seq。`0` 表示从未回收。
    pub fn reclaimed_through(&self) -> u64 {
        self.meta.reclaimed_through
    }

    /// 最后一条 seq（空日志为 0）。
    pub fn last_seq(&self) -> u64 {
        self.next_seq - 1
    }

    /// 当前持有的条目数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 追加一条（`seq` 由日志分配，调用方不必关心），并打上落账时间。
    /// 返回分配到的 `seq`。
    pub fn append(&mut self, kind: &str, text: &str, data: Option<serde_json::Value>) -> Result<u64, LogError> {
        Ok(self.append_at(kind, text, data, now_unix())?.seq)
    }

    /// 追加并返回**落账后的条目本身**（含 seq 与时间戳）。
    ///
    /// 上报批次要的就是这个：批里应当是"日志里真实的那一条"，而不是从事件重建的
    /// 影子条目——重建会把时间戳之类的东西丢掉，两处说法就不一致了。
    pub fn append_entry(
        &mut self,
        kind: &str,
        text: &str,
        data: Option<serde_json::Value>,
    ) -> Result<LogEntry, LogError> {
        self.append_at(kind, text, data, now_unix())
    }

    /// 指定时间的追加（测试与回放用）。
    pub fn append_at(
        &mut self,
        kind: &str,
        text: &str,
        data: Option<serde_json::Value>,
        at_unix: i64,
    ) -> Result<LogEntry, LogError> {
        let seq = self.next_seq;
        let entry = LogEntry {
            seq,
            kind: kind.to_string(),
            text: text.to_string(),
            data,
            at_unix,
        };
        append_line(&log_path(&self.dir, &self.session_id), &entry)?;
        self.entries.push_back(entry.clone());
        self.next_seq += 1;
        Ok(entry)
    }

    /// 取 `from_seq`（含）之后的条目（用于按需回拉精确序列）。
    pub fn since(&self, from_seq: u64) -> Vec<LogEntry> {
        self.entries
            .iter()
            .filter(|e| e.seq >= from_seq)
            .cloned()
            .collect()
    }

    /// 全部条目（快照/列表用）。
    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.iter().cloned().collect()
    }

    /// 回收 `through_seq`（含）及之前的条目，并推进水位线。
    ///
    /// 返回实际推进到的水位线。**只有主控已确认的段才应被回收**由调用方决定——
    /// 4.4 的保留策略落地时会在那里收紧；这里只保证「回收一定留下痕迹」。
    pub fn reclaim_through(&mut self, through_seq: u64) -> Result<u64, LogError> {
        if through_seq <= self.meta.reclaimed_through {
            return Ok(self.meta.reclaimed_through);
        }
        self.entries.retain(|e| e.seq > through_seq);
        self.meta.reclaimed_through = through_seq;
        self.persist_meta()?;
        rewrite_entries(&log_path(&self.dir, &self.session_id), &self.entries)?;
        Ok(self.meta.reclaimed_through)
    }

    /// 按年龄回收：把**整段早于** `cutoff_unix` 的前缀回收掉。
    ///
    /// 只回收连续前缀，绝不从中间挖洞（挖洞会让控制面看到一个永远补不上的缺口）。
    /// 年龄未知（`at_unix == 0`）的条目**不回收**——保守优先于省磁盘。
    /// 返回推进后的水位线（没有可回收段 → `None`）。
    pub fn reclaim_older_than(&mut self, cutoff_unix: i64) -> Result<Option<u64>, LogError> {
        let mut through = None;
        for entry in &self.entries {
            if entry.at_unix == 0 || entry.at_unix >= cutoff_unix {
                break;
            }
            through = Some(entry.seq);
        }
        match through {
            None => Ok(None),
            Some(seq) => {
                self.reclaim_through(seq)?;
                Ok(Some(seq))
            }
        }
    }

    /// 重置日志：纪元递增、条目清空、水位线归零。
    ///
    /// 语义上这是**一段新历史**，因此纪元必须变——控制面据此不会把新旧接起来。
    pub fn reset(&mut self) -> Result<u64, LogError> {
        self.meta.epoch += 1;
        self.meta.reclaimed_through = 0;
        self.entries.clear();
        self.next_seq = 1;
        self.persist_meta()?;
        rewrite_entries(&log_path(&self.dir, &self.session_id), &self.entries)?;
        Ok(self.meta.epoch)
    }

    /// 磁盘占用（本会话两个文件之和），用于节点存储上限判定。
    pub fn bytes_on_disk(&self) -> u64 {
        [log_path(&self.dir, &self.session_id), meta_path(&self.dir, &self.session_id)]
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum()
    }

    fn persist_meta(&self) -> Result<(), LogError> {
        let path = meta_path(&self.dir, &self.session_id);
        let tmp = path.with_extension("meta.tmp");
        let text = serde_json::to_string(&self.meta)
            .map_err(|e| LogError::io(format!("无法序列化日志元数据：{e}")))?;
        std::fs::write(&tmp, text)
            .map_err(|e| LogError::io(format!("无法写入 {}：{e}", tmp.display())))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| LogError::io(format!("无法落盘 {}：{e}", path.display())))
    }
}

/// 当前 unix 秒。
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn log_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.log.jsonl"))
}

fn meta_path(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.meta.json"))
}

fn read_meta(path: &Path) -> Result<Meta, LogError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| LogError::io(format!("{} 无法解析：{e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Meta::default()),
        Err(e) => Err(LogError::io(format!("无法读取 {}：{e}", path.display()))),
    }
}

fn read_entries(path: &Path) -> Result<VecDeque<LogEntry>, LogError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(VecDeque::new()),
        Err(e) => return Err(LogError::io(format!("无法读取 {}：{e}", path.display()))),
    };
    let mut out = VecDeque::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LogEntry>(line) {
            Ok(entry) => out.push_back(entry),
            // 半截行（崩溃在写中途）只丢那一行：日志是只追加的，丢尾部比整段不可读好。
            Err(e) => eprintln!(
                "warning: {}:{} 无法解析（已跳过）：{e}",
                path.display(),
                i + 1
            ),
        }
    }
    Ok(out)
}

fn append_line(path: &Path, entry: &LogEntry) -> Result<(), LogError> {
    use std::io::Write;
    let line = serde_json::to_string(entry)
        .map_err(|e| LogError::io(format!("无法序列化日志条目：{e}")))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| LogError::io(format!("无法打开 {}：{e}", path.display())))?;
    writeln!(file, "{line}").map_err(|e| LogError::io(format!("无法追加 {}：{e}", path.display())))
}

fn rewrite_entries(path: &Path, entries: &VecDeque<LogEntry>) -> Result<(), LogError> {
    let tmp = path.with_extension("jsonl.tmp");
    let mut text = String::new();
    for entry in entries {
        text.push_str(
            &serde_json::to_string(entry)
                .map_err(|e| LogError::io(format!("无法序列化日志条目：{e}")))?,
        );
        text.push('\n');
    }
    std::fs::write(&tmp, text)
        .map_err(|e| LogError::io(format!("无法写入 {}：{e}", tmp.display())))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| LogError::io(format!("无法落盘 {}：{e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn seq_is_monotonic_and_entries_keep_order() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        assert_eq!(log.epoch(), 1);
        assert_eq!(log.last_seq(), 0);
        assert_eq!(log.append("output", "a", None).unwrap(), 1);
        assert_eq!(log.append("output", "b", None).unwrap(), 2);
        assert_eq!(log.append("thinking", "c", None).unwrap(), 3);
        assert_eq!(log.last_seq(), 3);
        let seqs: Vec<u64> = log.entries().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert_eq!(log.entries()[1].text, "b");
    }

    #[test]
    fn entries_survive_reopen() {
        let d = dir();
        {
            let mut log = SessionLog::open(d.path(), "s-1").unwrap();
            log.append("output", "a", None).unwrap();
            log.append("output", "b", None).unwrap();
        }
        let log = SessionLog::open(d.path(), "s-1").unwrap();
        assert_eq!(log.last_seq(), 2, "重启后序列应继续");
        assert_eq!(log.entries().len(), 2);
        assert_eq!(log.epoch(), 1);
    }

    #[test]
    fn since_returns_the_requested_range() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        for i in 0..5 {
            log.append("output", &format!("t{i}"), None).unwrap();
        }
        let tail = log.since(3);
        assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![3, 4, 5]);
        assert!(log.since(6).is_empty());
    }

    #[test]
    fn reclaim_drops_entries_but_remembers_the_watermark() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        for i in 0..5 {
            log.append("output", &format!("t{i}"), None).unwrap();
        }
        assert_eq!(log.reclaim_through(3).unwrap(), 3);
        assert_eq!(log.reclaimed_through(), 3);
        assert_eq!(
            log.entries().iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![4, 5],
            "回收后只留未回收段"
        );
        // 水位线不回退。
        assert_eq!(log.reclaim_through(2).unwrap(), 3);
        // 重启后水位线仍在（控制面才能把缺口标成「已回收」而非 pending）。
        let log = SessionLog::open(d.path(), "s-1").unwrap();
        assert_eq!(log.reclaimed_through(), 3);
        assert_eq!(log.last_seq(), 5, "回收不应影响 seq 连续性");
    }

    #[test]
    fn reset_bumps_epoch_and_clears_history() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        log.append("output", "a", None).unwrap();
        log.reclaim_through(1).unwrap();
        let epoch = log.reset().unwrap();
        assert_eq!(epoch, 2, "重置即一段新历史 → 纪元必须变");
        assert_eq!(log.epoch(), 2);
        assert!(log.is_empty());
        assert_eq!(log.last_seq(), 0);
        assert_eq!(log.reclaimed_through(), 0);

        let reopened = SessionLog::open(d.path(), "s-1").unwrap();
        assert_eq!(reopened.epoch(), 2, "纪元应持久化");
    }

    #[test]
    fn bytes_on_disk_grows_with_entries() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        let before = log.bytes_on_disk();
        for i in 0..20 {
            log.append("output", &format!("line {i}"), None).unwrap();
        }
        assert!(log.bytes_on_disk() > before, "落盘体积应随条目增长");
    }

    #[test]
    fn torn_tail_line_is_skipped_not_fatal() {
        let d = dir();
        {
            let mut log = SessionLog::open(d.path(), "s-1").unwrap();
            log.append("output", "good", None).unwrap();
        }
        // 模拟崩溃留下的半截行。
        let path = log_path(d.path(), "s-1");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{\"seq\":2,\"kind\":\"outp");
        std::fs::write(&path, text).unwrap();

        let log = SessionLog::open(d.path(), "s-1").unwrap();
        assert_eq!(log.entries().len(), 1, "只丢坏行，其余仍可读");
        assert_eq!(log.last_seq(), 1);
    }
    #[test]
    fn entries_are_timestamped_at_append_time() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        log.append_at("output", "old", None, 1_000).unwrap();
        log.append("output", "now", None).unwrap();
        let entries = log.entries();
        assert_eq!(entries[0].at_unix, 1_000);
        assert!(entries[1].at_unix > 1_000, "默认打当前时间");
    }

    #[test]
    fn retention_reclaims_only_the_old_prefix() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        log.append_at("output", "a", None, 1_000).unwrap();
        log.append_at("output", "b", None, 1_100).unwrap();
        log.append_at("output", "c", None, 2_000).unwrap();
        log.append_at("output", "d", None, 2_100).unwrap();

        // 截止 1_500：只有前两条（连续前缀）该被回收。
        let watermark = log.reclaim_older_than(1_500).unwrap();
        assert_eq!(watermark, Some(2));
        assert_eq!(log.reclaimed_through(), 2);
        assert_eq!(
            log.entries().iter().map(|e| e.text.clone()).collect::<Vec<_>>(),
            vec!["c", "d"]
        );
        // 控制面据此把 <=2 的缺口标为「节点已回收」而不是 pending。
        assert_eq!(log.last_seq(), 4, "回收不影响 seq 连续性");
    }

    #[test]
    fn retention_never_holes_the_log() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        log.append_at("output", "old", None, 1_000).unwrap();
        log.append_at("output", "new", None, 5_000).unwrap();
        log.append_at("output", "old-again", None, 1_100).unwrap();

        // 第三条虽老，但它前面有一条新的：不挖洞 → 只回收第一条。
        let watermark = log.reclaim_older_than(2_000).unwrap();
        assert_eq!(watermark, Some(1));
        assert_eq!(log.entries().len(), 2, "绝不从中间挖洞");
    }

    #[test]
    fn retention_leaves_unknown_age_entries_alone() {
        let d = dir();
        let mut log = SessionLog::open(d.path(), "s-1").unwrap();
        log.append_at("output", "unknown-age", None, 0).unwrap();
        log.append_at("output", "old", None, 1_000).unwrap();
        // 第一条年龄未知 → 保守不动（它挡住了前缀）。
        assert_eq!(log.reclaim_older_than(2_000).unwrap(), None);
        assert_eq!(log.entries().len(), 2);
    }

    #[test]
    fn retention_is_survivable_across_reopen() {
        let d = dir();
        {
            let mut log = SessionLog::open(d.path(), "s-1").unwrap();
            log.append_at("output", "a", None, 1_000).unwrap();
            log.append_at("output", "b", None, 3_000).unwrap();
            log.reclaim_older_than(2_000).unwrap();
        }
        let log = SessionLog::open(d.path(), "s-1").unwrap();
        assert_eq!(log.reclaimed_through(), 1, "水位线持久化");
        assert_eq!(log.last_seq(), 2);
    }
}
