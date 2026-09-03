//! `sebas record --output fixture.jsonl` — ACP stdio 抓包（openspec/specs/replay-debug/spec.md）。
//!
//! 用当前配置（`acp.claude.path`/`args`）spawn agent，把用户终端的 stdin
//! 逐行转发给子进程 stdin、把子进程 stdout 逐行回显到终端；两个方向的
//! 每一行 JSON-RPC 同时追加到 fixture 文件，格式与 fake-claude 的
//! `--journal` 完全一致：`{"dir":"in"|"out","msg":{...}}`，每行一条。
//! 录好的 fixture 脱敏后即可进 `tests/fixtures/acp/` 供回放 harness 使用。
//!
//! 非 JSON 行原样转发（不录制）——agent 协议上是 NDJSON，但人手输入时
//! 难免打错；打错的行让 agent 自己报错，比 record 拒发更贴近真实交互。

use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use crate::config::Config;

pub struct RecordArgs {
    pub output: PathBuf,
    pub config: String,
    /// Extra args appended after the configured `acp.claude.args`
    /// (everything after `--` on the command line).
    pub agent_args: Vec<String>,
}

/// CLI entry: parse config, then record against the live terminal stdio.
pub async fn run(args: RecordArgs) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| anyhow::anyhow!("read config {}: {e}", args.config))?;
    let cfg = Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    record_with_io(&cfg, &args.agent_args, stdin, stdout, &args.output).await
}

/// Record one ACP session's stdio into `output`. Separated from `run` so
/// tests can drive it with in-memory streams instead of process stdio.
///
/// Lifecycle: user's EOF closes the child's stdin (a well-behaved agent
/// then exits); the child exiting (stdout EOF) ends the recording. Either
/// way the child is reaped and its exit status is reported.
pub async fn record_with_io<R, W>(
    cfg: &Config,
    agent_args: &[String],
    input: R,
    term_out: W,
    output: &Path,
) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let command = cfg.acp.command_for(cfg.acp.default_kind()).unwrap_or_default();
    let mut cmd = tokio::process::Command::new(command.first().cloned().unwrap_or_else(|| "claude".to_string()));
    cmd.args(command.iter().skip(1))
        .args(agent_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    let mut child = cmd
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn '{}': {e}", command.first().cloned().unwrap_or_else(|| "claude".to_string())))?;
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("child stdin piped"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("child stdout piped"))?;

    let file = std::sync::Arc::new(Mutex::new(
        tokio::fs::File::create(output)
            .await
            .map_err(|e| anyhow::anyhow!("create {}: {e}", output.display()))?,
    ));

    // in: terminal → child stdin (+ journal)
    let file_in = file.clone();
    let in_task = tokio::spawn(async move {
        let mut lines = BufReader::new(input).lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            child_stdin.write_all(line.as_bytes()).await?;
            child_stdin.write_all(b"\n").await?;
            child_stdin.flush().await?;
            append_journal(&file_in, "in", &line).await;
        }
        // EOF: dropping child_stdin closes the pipe so the agent exits.
        drop(child_stdin);
        Ok::<_, anyhow::Error>(())
    });

    // out: child stdout → terminal (+ journal)
    let file_out = file.clone();
    let mut out_task = tokio::spawn(async move {
        let mut term = term_out;
        let mut lines = BufReader::new(child_stdout).lines();
        while let Some(line) = lines.next_line().await? {
            term.write_all(line.as_bytes()).await?;
            term.write_all(b"\n").await?;
            term.flush().await?;
            append_journal(&file_out, "out", &line).await;
        }
        Ok::<_, anyhow::Error>(())
    });

    // The out task ends when the child closes stdout (i.e. it exited or is
    // exiting); then reap. The in task may still be blocked on user input —
    // abandon it, the process is going away.
    let out_result = (&mut out_task).await;
    in_task.abort();
    let status = child.wait().await?;
    out_result??;

    if !status.success() {
        anyhow::bail!("agent exited with {status}");
    }
    Ok(())
}

/// Append one journal line. Non-JSON input is forwarded but not recorded.
async fn append_journal(file: &std::sync::Arc<Mutex<tokio::fs::File>>, dir: &str, line: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        tracing::warn!(dir, "record: skipping non-JSON line");
        return;
    };
    let rec = serde_json::json!({"dir": dir, "msg": v});
    let mut s = serde_json::to_string(&rec).unwrap_or_default();
    s.push('\n');
    let mut f = file.lock().await;
    if let Err(e) = f.write_all(s.as_bytes()).await {
        tracing::warn!(?e, "record: journal write failed");
    }
    let _ = f.flush().await;
}
