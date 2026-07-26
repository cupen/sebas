use crate::session::{AcpCommand, AcpEvent, AcpSessionHandle};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub claude_path: String,
    pub claude_args: Vec<String>,
    pub work_dir: Option<String>,
}

pub struct AcpClient;

impl AcpClient {
    pub fn spawn(cfg: &SpawnConfig) -> std::io::Result<AcpSessionHandle> {
        let mut cmd = Command::new(&cfg.claude_path);
        cmd.args(&cfg.claude_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(ref wd) = cfg.work_dir {
            cmd.current_dir(wd);
        }
        let child: Child = cmd.spawn()?;
        Ok(handle_child(child))
    }
}

fn handle_child(mut child: Child) -> AcpSessionHandle {
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let stdin = child.stdin.take().expect("stdin piped");

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<AcpCommand>(64);
    let (evt_tx, evt_rx) = mpsc::channel::<AcpEvent>(256);

    // stdout reader → events
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<AcpEvent>(&line) {
                Ok(ev) => {
                    let _ = evt_tx.send(ev).await;
                }
                Err(e) => tracing::warn!(?e, raw=%line, "failed to parse acp stdout line"),
            }
        }
    });

    // stderr → tracing
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "acp_stderr", "{line}");
        }
    });

    // cmd_rx → stdin writer
    let stdin_task = tokio::spawn(async move {
        let mut s = stdin;
        while let Some(cmd) = cmd_rx.recv().await {
            match serde_json::to_string(&cmd) {
                Ok(mut json) => {
                    json.push('\n');
                    if let Err(e) = s.write_all(json.as_bytes()).await {
                        tracing::error!(?e, "failed to write to acp stdin");
                        break;
                    }
                    let _ = s.flush().await;
                }
                Err(e) => tracing::error!(?e, "failed to serialize acp command"),
            }
        }
    });

    AcpSessionHandle {
        child_id: format!("{:?}", child.id()),
        child: Some(child),
        cmd_tx,
        evt_rx: Arc::new(Mutex::new(evt_rx)),
        _stdin_task: stdin_task,
    }
}