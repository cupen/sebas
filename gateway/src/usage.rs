//! usage record → jsonl sink（Task 8，spec §4.5）。
//!
//! `UsageSink::spawn_writer` 起 mpsc(256) + tokio task 追加写 jsonl（先建父目录）。
//! `record` 用 `try_send`，满则 warn 丢弃。`UsageRecord` 含本次请求的元数据与
//! token 计数；`key` 字段记下游 key 的 `name`（非 key 本体，安全约束）。

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// 一次请求的用量记录。`key` = 下游 key 的 `name` 字段（绝不记 key 本体）。
/// `error` 留给网关侧失败（如 connect 502）；上游 4xx/5xx 不填 `error`（status
/// 字段承载其错误语义）。token 字段为 `None` 表示本次未观测到该计数（如
/// 解析失败、流被截断、或上游错误响应无 usage）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub ts: String,
    pub key: String,
    pub protocol: String,
    pub model: Option<String>,
    pub provider: String,
    pub upstream_model: Option<String>,
    pub status: u16,
    pub latency_ms: u64,
    pub ttft_ms: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
    pub error: Option<String>,
}

/// jsonl 写入器句柄。`Clone` 因 `mpsc::Sender` 可克隆（AppState 需 Clone）。
/// drop 不等 sink 关闭——后台 task 在 `recv → None` 时自然退出。
#[derive(Clone)]
pub struct UsageSink {
    tx: mpsc::Sender<UsageRecord>,
}

/// 后台 writer task 的容量。满则 `record` 用 `try_send` 丢弃并 warn。
const CHANNEL_CAPACITY: usize = 256;

impl UsageSink {
    /// 起后台 task 追加写 jsonl。先建父目录（同步，一次性，启动时）。
    /// 失败 → io::Error 转嫁调用方（build_state 映射为 Config 错误拒绝启动）。
    ///
    /// `tokio::spawn` 要求调用线程处于 tokio 运行时上下文。`build_state`
    /// 在 `run`（async）/测试（`#[tokio::test]`）内被调用，运行时恒存在。
    pub fn spawn_writer(path: impl AsRef<Path>) -> io::Result<Self> {
        let path: PathBuf = path.as_ref().to_path_buf();
        // 父目录先建：避免 writer task 反复重建已删父目录；也避免 task
        // 启动时 open 失败导致所有 record 丢弃。空 parent（相对路径文件）
        // 跳过 create_dir_all（"." 不需要建）。
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && parent != Path::new(".")
        {
            std::fs::create_dir_all(parent)?;
        }
        let (tx, mut rx) = mpsc::channel::<UsageRecord>(CHANNEL_CAPACITY);
        tokio::spawn(async move {
            while let Some(rec) = rx.recv().await {
                write_record(&path, &rec).await;
            }
        });
        Ok(UsageSink { tx })
    }

    /// 投递一条 record。mpsc(256) 满则 warn 丢弃（warn 不含 key 材料——
    /// key 材料只存在于 record 内，warn 串恒为静态字面量）。channel 关闭
    /// （writer task 已退出）同样 warn 丢弃。
    pub fn record(&self, rec: UsageRecord) {
        if let Err(e) = self.tx.try_send(rec) {
            match e {
                mpsc::error::TrySendError::Full(_) => {
                    tracing::warn!(
                        "usage sink channel full (cap {CHANNEL_CAPACITY}); dropping record"
                    );
                }
                mpsc::error::TrySendError::Closed(_) => {
                    tracing::warn!("usage sink writer closed; dropping record");
                }
            }
        }
    }
}

/// 把一条 record 序列化为 JSON 追加写入 jsonl。序列化/写失败 → warn 丢弃
/// （不阻塞调用方；usage 是统计旁路，绝不应阻断转发）。
async fn write_record(path: &Path, rec: &UsageRecord) {
    use tokio::io::AsyncWriteExt;
    let line = match serde_json::to_string(rec) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "usage record serialize failed; dropping");
            return;
        }
    };
    // 每条记录独立 open-append。P0 简单稳健：文件被外部 rotate/截断时下次
    // 写入会重建；不必持有 handle 跟踪文件生命周期。网关 RPM 量级下开销
    // 可忽略（上游响应毫秒级，IO 微秒级）。
    let mut f = match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "usage jsonl open failed; dropping record");
            return;
        }
    };
    if let Err(e) = f.write_all(line.as_bytes()).await {
        tracing::warn!(error = %e, "usage jsonl write failed; dropping record");
        return;
    }
    // 行尾换行独立写一次：即便 line 内含换行（理论不会），jsonl 边界仍清晰。
    let _ = f.write_all(b"\n").await;
    let _ = f.flush().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    // ---------------- spawn_writer 写两条 record → 两行合法 jsonl ----------------

    #[tokio::test]
    async fn spawn_writer_writes_records_as_jsonl() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("usage.jsonl");
        let sink = UsageSink::spawn_writer(&path).expect("spawn_writer");

        let rec1 = UsageRecord {
            ts: "2026-08-07T00:00:00Z".into(),
            key: "alice".into(),
            protocol: "anthropic".into(),
            model: Some("claude-sonnet".into()),
            provider: "anthropic".into(),
            upstream_model: Some("anthropic.claude-sonnet-4".into()),
            status: 200,
            latency_ms: 123,
            ttft_ms: Some(45),
            input_tokens: Some(10),
            output_tokens: Some(50),
            cache_read_tokens: Some(5),
            cache_creation_tokens: Some(2),
            error: None,
        };
        let mut rec2 = rec1.clone();
        rec2.key = "bob".into();
        rec2.status = 500;
        rec2.error = Some("upstream connect failed".into());

        sink.record(rec1);
        sink.record(rec2);

        // 轮询文件直到两行出现（writer 异步，需带超时重试）。
        let lines = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                let lines: Vec<String> = content
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect();
                if lines.len() >= 2 {
                    return lines;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("two records written within 2s");

        // 每行合法 JSON，字段齐全
        let v1: serde_json::Value = serde_json::from_str(&lines[0]).expect("line 1 valid JSON");
        let v2: serde_json::Value = serde_json::from_str(&lines[1]).expect("line 2 valid JSON");

        assert_eq!(v1["key"], "alice");
        assert_eq!(v1["protocol"], "anthropic");
        assert_eq!(v1["model"], "claude-sonnet");
        assert_eq!(v1["provider"], "anthropic");
        assert_eq!(v1["upstream_model"], "anthropic.claude-sonnet-4");
        assert_eq!(v1["status"], 200);
        assert_eq!(v1["latency_ms"], 123);
        assert_eq!(v1["ttft_ms"], 45);
        assert_eq!(v1["input_tokens"], 10);
        assert_eq!(v1["output_tokens"], 50);
        assert_eq!(v1["cache_read_tokens"], 5);
        assert_eq!(v1["cache_creation_tokens"], 2);
        assert!(v1.get("error").is_some() && v1["error"].is_null());

        assert_eq!(v2["key"], "bob");
        assert_eq!(v2["status"], 500);
        assert_eq!(v2["error"], "upstream connect failed");
    }
}
