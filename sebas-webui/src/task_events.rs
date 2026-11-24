//! 每项目 beads 事件泵（agent-task-center 4.1~4.4）.
//!
//! beads journal 是 pull 模型（`bd events tail --since <seq>`），没有推送
//! 能力；本模块把「拉」收敛成一条 per-project 的广播流：
//!
//! - 每个打开了任务面板的项目一条 tokio 任务，每 [`POLL_INTERVAL`] 调一次
//!   `bd events tail --since <last_seq>`，单次 [`EVENTS_TAIL_TIMEOUT`] 超时；
//! - 事件按 seq 升序进 [`broadcast`] 通道，WS 连接各自订阅（fan-out）；
//! - `last_seq` 在项目内**跨泵重启保留**（`Arc<AtomicI64>`）——订阅者掉线
//!   不影响游标，新连接从同一水位继续，不重放也不跳号；
//! - 无订阅者即退出（每 tick 检查 `receiver_count`），进程内不会留下空转的
//!   `bd` 子进程；
//! - 单次失败只记日志、下一 tick 用**同一个** `last_seq` 重试（spec
//!   「Polling recovers from a transient failure」）；journal 截断
//!   （`events_journal_truncated`）按 floor 重新基线，后续缺口由手动
//!   Refresh（全量 `bd list`）收敛。

use crate::beads_client::{BeadsClient, BeadsError, BeadsEventRecord, EVENTS_TAIL_TIMEOUT};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::Mutex;
use tokio::sync::broadcast;

/// 轮询间隔（design D5：300ms；p95 端到端 < 500ms）。
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(300);

/// 广播缓冲。任务事件是「整份 issue 快照」的替换语义，落后即丢帧由
/// `task_resync` 收敛——缓冲不需要很大。
pub const TASK_EVENT_CAPACITY: usize = 256;

/// 泵产出的一条事件。
///
/// `record` 装箱：journal 记录含完整 issue 快照（数百字节），枚举变体
/// 按最宽成员定长，装箱后广播通道的每槽内存回到可忽略量级。
#[derive(Debug, Clone, PartialEq)]
pub enum TaskEvent {
    /// journal 新记录（`issue` 是变更后的完整快照）。
    Updated {
        project_id: String,
        record: Box<BeadsEventRecord>,
    },
    /// 订阅者落后广播（丢了帧）：让前端全量重取该项目的任务列表。
    Resync { project_id: String },
}

struct PumpHandle {
    tx: broadcast::Sender<TaskEvent>,
    /// 项目级水位：泵重启 / 手动 Refresh 重新基线都写这里。
    last_seq: Arc<AtomicI64>,
}

/// 每项目事件泵的注册表（`WebUiState` 持有）。
pub struct TaskEventHub {
    client: BeadsClient,
    pumps: Mutex<HashMap<String, PumpHandle>>,
    /// 已完成惰性初始化的项目（P3-2）：spec 是「首次访问」跑一次，而不是
    /// 每个任务端点都多 spawn 一个 `bd config set`。命中缓存仍 stat
    /// `.beads/`——目录被删时重新初始化，不把陈旧标记当现状。
    ensured: Mutex<std::collections::HashSet<String>>,
}

impl Default for TaskEventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskEventHub {
    /// 生产构造：`bd` 二进制经 `SEBAS_BD_BIN` 或 `PATH` 发现。
    pub fn new() -> Self {
        Self::with_client(BeadsClient::new())
    }

    /// 显式客户端（测试指向假桩）。
    pub fn with_client(client: BeadsClient) -> Self {
        Self {
            client,
            pumps: Mutex::new(HashMap::new()),
            ensured: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// 共享的 beads 客户端（HTTP handler 复用它调 `bd`）。
    pub fn client(&self) -> &BeadsClient {
        &self.client
    }

    /// 惰性初始化（spec「首次访问」）：命中缓存且 `.beads/` 仍在即返回，
    /// 否则跑一次 `ensure_initialized` 并记住。
    pub async fn ensure_ready(
        &self,
        project_id: &str,
        project_dir: &Path,
    ) -> Result<(), BeadsError> {
        {
            let ensured = self.ensured.lock().await;
            if ensured.contains(project_id) && project_dir.join(".beads").exists() {
                return Ok(());
            }
        }
        self.client.ensure_initialized(project_dir).await?;
        self.ensured.lock().await.insert(project_id.to_string());
        Ok(())
    }

    /// 订阅一个项目的事件流；首个订阅者启动泵，后续订阅者复用同一条泵。
    ///
    /// 已有泵但**无接收者**（上一轮订阅者都已掉线、泵即将退出）时替换成
    /// 新泵并从保留的 `last_seq` 继续——订阅者永远拿不到死通道。
    pub async fn subscribe(
        &self,
        project_id: &str,
        project_dir: &Path,
    ) -> broadcast::Receiver<TaskEvent> {
        let mut pumps = self.pumps.lock().await;
        if let Some(existing) = pumps.get(project_id)
            && existing.tx.receiver_count() > 0
        {
            return existing.tx.subscribe();
        }
        let last_seq = pumps
            .get(project_id)
            .map(|p| p.last_seq.clone())
            .unwrap_or_else(|| Arc::new(AtomicI64::new(0)));
        let (tx, rx) = broadcast::channel(TASK_EVENT_CAPACITY);
        let hub_tx = tx.clone();
        let dir: PathBuf = project_dir.to_path_buf();
        let pid = project_id.to_string();
        let client = self.client.clone();
        let seq = last_seq.clone();
        tokio::spawn(async move { run_pump(client, pid, dir, hub_tx, seq).await });
        pumps.insert(project_id.to_string(), PumpHandle { tx, last_seq });
        rx
    }

    /// 手动 Refresh 的重新基线：把项目水位钉到 journal 当前 head（`seq`），
    /// 正在跑的泵下一 tick 从这个水位继续（不再重放旧记录）。
    pub async fn reset_baseline(&self, project_id: &str, seq: i64) {
        let pumps = self.pumps.lock().await;
        if let Some(pump) = pumps.get(project_id) {
            pump.last_seq.store(seq, Ordering::SeqCst);
        }
    }
}

/// 单项目轮询循环：无订阅者即退出；失败不改游标（下一 tick 原样重试）。
async fn run_pump(
    client: BeadsClient,
    project_id: String,
    project_dir: PathBuf,
    tx: broadcast::Sender<TaskEvent>,
    last_seq: Arc<AtomicI64>,
) {
    loop {
        if tx.receiver_count() == 0 {
            return;
        }
        let since = last_seq.load(Ordering::SeqCst);
        match client
            .events_tail(&project_dir, since, EVENTS_TAIL_TIMEOUT)
            .await
        {
            Ok(records) => {
                for record in records {
                    // 严格增量：`--since` 之外的防御（CLI 契约漂移时不重放）。
                    if record.seq <= since {
                        continue;
                    }
                    last_seq.store(record.seq, Ordering::SeqCst);
                    let _ = tx.send(TaskEvent::Updated {
                        project_id: project_id.clone(),
                        record: Box::new(record),
                    });
                }
            }
            // journal 被保留期裁剪：从 floor 继续（之前的序号永久不可读），
            // 缺口由手动 Refresh 的全量重载收敛。
            Err(BeadsError::JournalTruncated { floor, .. }) => {
                tracing::warn!(
                    project_id = %project_id,
                    floor,
                    "beads events journal 已截断；从 floor 继续，缺口请用 Refresh 重新基线"
                );
                last_seq.store(floor.saturating_sub(1), Ordering::SeqCst);
            }
            // 暂时性失败（kill bd / 超时 / 写冲突）：游标不动，下一 tick
            // 用同一个 since 重试——恢复后不跳事件。
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id,
                    since,
                    %error,
                    "beads events 轮询失败；同一 last_seq 重试"
                );
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn fake_bd(dir: &Path, script: &str) -> PathBuf {
        let bin = dir.join("bd");
        let mut f = std::fs::File::create(&bin).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        f.sync_all().unwrap();
        drop(f);
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn event_line(seq: i64) -> String {
        format!(
            r#"{{"seq":{seq},"ts":"2026-11-24T12:00:0{seq}Z","op":"create","issue_id":"tsk-{seq}","issue":{{"id":"tsk-{seq}","title":"t{seq}","status":"open"}}}}"#
        )
    }

    /// 4.1 验收：订阅即启动泵，事件按 seq 升序经广播到达；两次轮询之间
    /// 拿到的游标是**严格增量**（第二次 `--since` 是首个事件的 seq）。
    #[tokio::test]
    async fn pump_delivers_journal_records_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let calls = project.join("calls.log");
        let calls_str = calls.display().to_string();
        // 每次调用输出 1~2 条记录（第二 tick 只输出 seq 2），并把参数记进
        // calls.log 供断言 `--since` 的增量语义。
        let script = format!(
            r#"#!/bin/sh
echo "$@" >> {calls_str}
n=$(wc -l < {calls_str})
if [ "$n" -eq 1 ]; then
  echo '{a}'
  echo '{b}'
else
  echo '{b}'
fi
"#,
            a = event_line(1),
            b = event_line(2),
        );
        let bin = fake_bd(dir.path(), &script);
        let hub = TaskEventHub::with_client(BeadsClient::with_bin(bin));
        let mut rx = hub.subscribe("proj-1", &project).await;

        let first = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("first event")
            .unwrap();
        let TaskEvent::Updated { project_id, record } = first else {
            panic!("expected an Updated event");
        };
        assert_eq!(project_id, "proj-1");
        assert_eq!(record.seq, 1);
        assert_eq!(record.issue.as_ref().unwrap().id, "tsk-1");

        let second = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("second event")
            .unwrap();
        let TaskEvent::Updated { record, .. } = second else {
            panic!("expected an Updated event");
        };
        assert_eq!(record.seq, 2, "seq 升序、不重放");

        // 等下一 tick 真正落一次 `--since 2` 的调用（首批两条事件在同一
        // 轮询内到达，第二次调用的记录可能还没写盘）。
        tokio::time::sleep(POLL_INTERVAL * 3).await;
        // 增量：第二次调用的 --since 是 2（首个事件的 seq），不是 0。
        let invocations = std::fs::read_to_string(&calls).unwrap();
        let second_call = invocations.lines().nth(1).unwrap();
        assert!(
            second_call.contains("--since 2"),
            "轮询必须用 --since 严格增量: {second_call}"
        );
    }

    /// 4.4 验收：单次 `bd` 失败（exit 1）只记日志，下一 tick 用同一
    /// `last_seq` 重试——恢复后事件不跳号。
    #[tokio::test]
    async fn transient_failure_retries_same_last_seq_without_skipping() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let calls = project.join("calls.log");
        let calls_str = calls.display().to_string();
        let script = format!(
            r#"#!/bin/sh
echo "$@" >> {calls_str}
n=$(wc -l < {calls_str})
if [ "$n" -eq 1 ]; then
  echo '{a}'
  exit 0
fi
if [ "$n" -eq 2 ]; then
  echo "transient write conflict" >&2
  exit 1
fi
echo '{b}'
"#,
            a = event_line(1),
            b = event_line(2),
        );
        let bin = fake_bd(dir.path(), &script);
        let hub = TaskEventHub::with_client(BeadsClient::with_bin(bin));
        let mut rx = hub.subscribe("proj-1", &project).await;

        let mut seqs = Vec::new();
        while seqs.len() < 2 {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("event before timeout")
                .unwrap();
            let TaskEvent::Updated { record, .. } = event else {
                continue;
            };
            seqs.push(record.seq);
        }
        assert_eq!(seqs, vec![1, 2], "失败不吞事件、不跳号");

        // 失败那次之后的调用仍以 --since 1（未推进的水位）重试。
        let invocations = std::fs::read_to_string(&calls).unwrap();
        let third_call = invocations.lines().nth(2).unwrap();
        assert!(
            third_call.contains("--since 1"),
            "失败后必须用同一 last_seq 重试: {third_call}"
        );
    }

    /// P3-2 验收：惰性初始化只在**首次访问**发生——第二次 `ensure_ready`
    /// 命中缓存，不再 spawn `bd init` / `bd config set`。
    #[tokio::test]
    async fn ensure_ready_runs_once_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let calls = project.join("calls.log");
        let calls_str = calls.display().to_string();
        // init 需要建出 .beads/（ensure_ready 的缓存命中条件之一）。
        let script = format!(
            r#"#!/bin/sh
echo "$@" >> {calls_str}
case "$*" in
  *init*) mkdir -p .beads;;
esac
"#
        );
        let bin = fake_bd(dir.path(), &script);
        let hub = TaskEventHub::with_client(BeadsClient::with_bin(bin));

        hub.ensure_ready("proj-1", &project).await.unwrap();
        let after_first = std::fs::read_to_string(&calls).unwrap().lines().count();
        assert_eq!(after_first, 2, "首次：init + config set 各一次");

        hub.ensure_ready("proj-1", &project).await.unwrap();
        hub.ensure_ready("proj-1", &project).await.unwrap();
        let after_more = std::fs::read_to_string(&calls).unwrap().lines().count();
        assert_eq!(after_more, after_first, "后续访问不重复 spawn bd");

        // `.beads/` 被删 → 缓存失效，重新初始化。
        std::fs::remove_dir_all(project.join(".beads")).unwrap();
        hub.ensure_ready("proj-1", &project).await.unwrap();
        let after_delete = std::fs::read_to_string(&calls).unwrap().lines().count();
        assert!(after_delete > after_more, "目录被删必须重新初始化");
    }

    /// 4.1 生命周期：最后一个订阅者掉线后泵退出（不空转 spawn `bd`）。
    #[tokio::test]
    async fn pump_exits_after_last_subscriber_drops() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let calls = project.join("calls.log");
        let calls_str = calls.display().to_string();
        let script = format!("#!/bin/sh\necho \"$@\" >> {calls_str}\n");
        let bin = fake_bd(dir.path(), &script);
        let hub = TaskEventHub::with_client(BeadsClient::with_bin(bin));
        let rx = hub.subscribe("proj-1", &project).await;
        // 泵至少跑过一次（receiver 在场）。
        tokio::time::sleep(POLL_INTERVAL).await;
        assert!(!std::fs::read_to_string(&calls).unwrap().is_empty());
        drop(rx);
        // 给泵一个 tick 观察空订阅者并退出，之后不再有新调用。
        tokio::time::sleep(POLL_INTERVAL * 4).await;
        let after = std::fs::read_to_string(&calls).unwrap().lines().count();
        tokio::time::sleep(POLL_INTERVAL * 4).await;
        let later = std::fs::read_to_string(&calls).unwrap().lines().count();
        assert_eq!(after, later, "无订阅者后泵必须退出");
    }
}
