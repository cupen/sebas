//! headless 冒烟入口（task 6.1）：sebas-agent Phase 1a 唯一的人工验证面。
//!
//! 用法：
//! ```text
//! # 单条 prompt：跑一轮后退出
//! SEBAS_AGENT_PROVIDER_BASE_URL=https://api.anthropic.com \
//! SEBAS_AGENT_PROVIDER_API_KEY=sk-ant-... \
//! cargo run -p sebas-agent --example agent-dev -- --workdir . "列出当前目录的 rust 文件"
//!
//! # 交互 shell（--shell）：每行一个 prompt，便于人工测试 agent 交互
//! cargo run -p sebas-agent --example agent-dev -- --shell
//! ```
//!
//! shell 命令（刻意保持最小，仅为方便测试）：
//! - 直接输入文本 = 提交 prompt（turn 进行中则排队）
//! - `/cancel` 取消当前 turn（验证 C7 取消安全）
//! - `/quit` / `/exit` / Ctrl-D 退出
//!
//! 端点选择（design N9）：默认直连 provider（Anthropic Messages 兼容端点）；
//! 设置 `SEBAS_AGENT_GATEWAY_URL` 则改走本地 gateway（可选路径，wire 协议相同）。
//! 事件流打印到 stderr；不改 CLI 命令表。

use sebas_agent::llm::anthropic::AnthropicMessagesClient;
use sebas_agent::session::{AgentEvent, SessionConfig, SessionHandle, SessionManager};
use sebas_agent::tools::ToolRegistry;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::broadcast;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn print_help() {
    eprintln!("usage: cargo run -p sebas-agent --example agent-dev -- [--workdir DIR] [--model MODEL] [--record FILE] [--shell] [PROMPT...]");
    eprintln!();
    eprintln!("  --shell       interactive shell for testing agent interaction");
    eprintln!("  --record FILE write all events as JSONL for later replay/debugging");
    eprintln!();
    eprintln!("env (direct provider, default):");
    eprintln!("  SEBAS_AGENT_PROVIDER_BASE_URL  (default: https://api.anthropic.com)");
    eprintln!("  SEBAS_AGENT_PROVIDER_API_KEY   (required unless gateway is set)");
    eprintln!("  SEBAS_AGENT_MODEL              (default: claude-sonnet-4-5)");
    eprintln!("env (optional gateway):");
    eprintln!("  SEBAS_AGENT_GATEWAY_URL        (e.g. http://127.0.0.1:8787)");
    eprintln!("  SEBAS_AGENT_GATEWAY_AUTH       (default: sk-gw-local-dev)");
}

/// 将事件记录为 JSONL 的工具。
struct Recorder {
    inner: Mutex<BufWriter<File>>,
}

impl Recorder {
    fn new(path: &PathBuf) -> Self {
        let file = File::create(path).unwrap_or_else(|e| {
            eprintln!("[agent-dev] failed to open record file {path:?}: {e}");
            std::process::exit(2);
        });
        Self {
            inner: Mutex::new(BufWriter::new(file)),
        }
    }

    fn record(&self, ev: &AgentEvent) {
        if let Ok(line) = serde_json::to_string(ev) {
            if let Ok(mut w) = self.inner.lock() {
                let _ = writeln!(w, "{line}");
            }
        }
    }

    fn record_prompt(&self, prompt: &str) {
        if let Ok(mut w) = self.inner.lock() {
            let _ = writeln!(w, "# prompt: {prompt}");
        }
    }

    fn flush(&self) {
        if let Ok(mut w) = self.inner.lock() {
            let _ = w.flush();
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}

/// 打印事件流；记录到 recorder（如有）；返回是否遇到 terminal 错误。
fn print_event(ev: &AgentEvent, recorder: Option<&Recorder>) -> bool {
    if let Some(r) = recorder {
        r.record(ev);
    }
    match ev {
        AgentEvent::TextDelta { delta, .. } | AgentEvent::ThinkingDelta { delta, .. } => {
            eprint!("{delta}");
            false
        }
        AgentEvent::ToolStart { tool_name, args, .. } => {
            eprintln!("\n[tool] {tool_name} {args}");
            false
        }
        AgentEvent::ToolProgress { .. } => false,
        AgentEvent::ToolEnd {
            tool_name, result, ..
        } => {
            eprintln!("\n[tool end] {tool_name}: {}", truncate(result, 400));
            false
        }
        AgentEvent::PermissionRequest {
            request_id,
            tool_name,
            args,
            reason,
            ..
        } => {
            eprintln!("\n[permission] {tool_name} {args} — {reason} (request {request_id})");
            false
        }
        AgentEvent::ToolPolicy {
            tool_name, outcome, ..
        } => {
            eprintln!("\n[policy] {tool_name}: {outcome}");
            false
        }
        AgentEvent::ToolFinish { tool_name, ok, .. } => {
            eprintln!("\n[tool finish] {tool_name} ok={ok}");
            false
        }
        AgentEvent::SessionSummary { model_calls, tool_calls, turn_ms, .. } => {
            eprintln!("\n[summary] model={model_calls} tools={tool_calls} {turn_ms}ms");
            false
        }
        AgentEvent::Finished { .. } => {
            eprintln!("\n[finished]");
            false
        }
        AgentEvent::Error {
            message, terminal, ..
        } => {
            eprintln!("\n[error terminal={terminal}] {message}");
            *terminal
        }
    }
}

/// 从 stdin 逐行读入并推送到 channel（阻塞线程，不阻塞运行时）。
async fn stdin_lines() -> tokio::sync::mpsc::Receiver<String> {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::task::spawn_blocking(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if tx.blocking_send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    rx
}

/// 交互 shell（--shell）：stdin 行 + 事件流的 select 循环。
/// turn 进行中仍可输入：/cancel 即时生效，新 prompt 由会话层串行排队；
/// /quit（或 EOF）等当前在途 turn 收尾后再退出，保证输出完整。
async fn run_shell(
    handle: &SessionHandle,
    mut rx: broadcast::Receiver<AgentEvent>,
    recorder: Option<&Recorder>,
) {
    eprintln!(
        "[agent-dev] shell ready (session {}) — type a prompt; /cancel cancels the current turn; /quit exits",
        handle.key
    );
    let mut lines = stdin_lines().await;
    // 每提交一条 prompt，会话层最终回一个终态事件（Finished / Error）。
    let mut in_flight = 0usize;
    let mut quit = false;
    loop {
        if quit && in_flight == 0 {
            eprintln!("[agent-dev] bye");
            break;
        }
        tokio::select! {
            line = lines.recv() => {
                let Some(raw) = line else {
                    quit = true; // EOF（Ctrl-D）：等在途 turn 收尾
                    continue;
                };
                let line = raw.trim();
                if line.is_empty() {
                    continue;
                }
                match line {
                    "/quit" | "/exit" | "/q" => {
                        quit = true;
                        if in_flight > 0 {
                            eprintln!("[agent-dev] quitting after {in_flight} in-flight turn(s)…");
                        }
                    }
                    "/cancel" => {
                        handle.cancel().await;
                        eprintln!("[agent-dev] cancel requested");
                    }
                    other => {
                        eprintln!("[agent-dev] » {other}");
                        in_flight += 1;
                        handle.prompt(other).await;
                    }
                }
            }
            ev = rx.recv() => {
                match ev {
                    Ok(ev) => {
                        let terminal = matches!(
                            ev,
                            AgentEvent::Finished { .. } | AgentEvent::Error { .. }
                        );
                        // terminal 错误 = 会话不可恢复，shell 退出
                        if print_event(&ev, recorder) {
                            break;
                        }
                        if terminal {
                            in_flight = in_flight.saturating_sub(1);
                            if !quit {
                                eprintln!("[agent-dev] ready for next prompt");
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("\n[agent-dev] lagged, skipped {n} events");
                    }
                    Err(_) => break, // 会话任务结束
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut workdir = PathBuf::from(".");
    let mut model: Option<String> = None;
    let mut shell = false;
    let mut record: Option<PathBuf> = None;
    let mut prompt_parts: Vec<String> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--workdir" => {
                workdir = PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--workdir needs a value");
                    std::process::exit(2);
                }));
            }
            "--model" => {
                model = Some(args.next().unwrap_or_else(|| {
                    eprintln!("--model needs a value");
                    std::process::exit(2);
                }));
            }
            "--record" => {
                record = Some(PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--record needs a file path");
                    std::process::exit(2);
                })));
            }
            "--shell" => shell = true,
            "--help" | "-h" => {
                print_help();
                return;
            }
            _ => prompt_parts.push(a),
        }
    }

    let recorder = record.as_ref().map(Recorder::new);

    // LLM 通道（design N9）：直连 provider 默认，gateway 可选。
    let client = if let Ok(url) = std::env::var("SEBAS_AGENT_GATEWAY_URL") {
        let auth = env_or("SEBAS_AGENT_GATEWAY_AUTH", "sk-gw-local-dev");
        eprintln!("[agent-dev] via gateway {url}");
        AnthropicMessagesClient::gateway(url, auth)
    } else {
        let base = env_or("SEBAS_AGENT_PROVIDER_BASE_URL", "https://api.anthropic.com");
        let key = env_or("SEBAS_AGENT_PROVIDER_API_KEY", "");
        if key.is_empty() {
            eprintln!(
                "[agent-dev] missing SEBAS_AGENT_PROVIDER_API_KEY \
                 (or set SEBAS_AGENT_GATEWAY_URL to go through the gateway)"
            );
            std::process::exit(2);
        }
        eprintln!("[agent-dev] direct provider {base}");
        AnthropicMessagesClient::direct_provider(base, key)
    };

    let model = model.unwrap_or_else(|| env_or("SEBAS_AGENT_MODEL", "claude-sonnet-4-5"));
    eprintln!("[agent-dev] workdir {} model {model}", workdir.display());

    let manager = SessionManager::new(
        Arc::new(client),
        ToolRegistry::new(Duration::from_secs(120)),
        SessionConfig::default(),
    );
    let handle = manager.create_session(workdir);
    let rx = handle.subscribe();

    if shell {
        run_shell(&handle, rx, recorder.as_ref()).await;
        if let Some(r) = recorder.as_ref() {
            r.flush();
        }
        return;
    }

    let prompt = prompt_parts.join(" ");
    if prompt.is_empty() {
        print_help();
        std::process::exit(2);
    }

    let recorder_ref = recorder.as_ref();
    let mut rx = rx;
    eprintln!("[agent-dev] session {} — prompt: {prompt}", handle.key);
    if let Some(r) = recorder_ref {
        r.record_prompt(&prompt);
    }
    handle.prompt(prompt).await;

    while let Ok(ev) = rx.recv().await {
        if print_event(&ev, recorder_ref) {
            break;
        }
        if matches!(ev, AgentEvent::Finished { .. }) {
            break;
        }
    }
    if let Some(r) = recorder_ref {
        r.flush();
        if let Some(path) = record.as_ref() {
            eprintln!("[agent-dev] events recorded to {}", path.display());
        }
    }
}
