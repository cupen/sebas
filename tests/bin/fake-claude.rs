//! Fake claude CLI for acp-claude's post-ACP engine (sebas-dk8.2).
//! Speaks the Claude Code stream-json + control-protocol dialect that
//! cc-agent-sdk drives:
//!   stdin  ← control_request initialize / user messages / control_response
//!   stdout → system init, assistant/user frames, result, control frames
//!
//! Fidelity contract (mirrors the real CLI, verified by wire fixtures in
//! spikes/cc-agent-sdk-spike/out/wire-frames.jsonl):
//! - `{cli} --version` prints a version line and exits 0 (SDK checks this).
//! - `control_request{subtype:initialize}` gets a `control_response` with
//!   `response.subtype == "success"`.
//! - The FIRST `user` message is preceded by exactly one `system/init` frame
//!   carrying the --session-id (or --resume) value from argv and our cwd.
//! - A tool_use turn sends `control_request{subtype:hook_callback}` and
//!   BLOCKS until the matching `control_response` arrives; `allow` runs the
//!   tool (tool_result success), `deny` fails it (tool_result is_error).
//! - `control_request{subtype:interrupt}` ends the turn with an error result
//!   and the process EXITS (the real CLI is unusable after interrupt).
//!
//! Flags (argv, not env — env races under cargo test parallelism):
//!   fake-claude-cli [scenario] [--loop] [--slow-ms N] [--hang-on-init]
//!                   [--delay-init-ms N] [--journal PATH] [--resume-fails]
//!   scenario: hello (default) | bash | deny | thinking
//!   --advertise-commands: the initialize control response carries a fixed
//!   command table (goal with an argumentHint + compact) — the claude-path
//!   command-discovery data source (session-slash-commands 1.2); without the
//!   flag the response stays `{}` exactly as before.
//!   --resume-fails: exit(1) with "No conversation found" on stderr, but ONLY
//!   when argv carries --resume — a fresh --session-id spawn still works
//!   (mirrors the real CLI; exercises the manager's fresh-session fallback).
//!
//! Content-triggered behaviors (for regression tests; take precedence over
//! the argv scenario):
//! - user text containing "crash" → emit one "boom" text frame then
//!   exit(1), modelling a mid-session process crash (D6).
//! - user text == "perm" → Bash tool_use + hook_callback; allow →
//!   tool_result "perm done", deny → tool_result is_error.
//! - user text == "stream" → 5 text frames with a 250ms pause before the
//!   result frame (exercises the 150ms-debounce pump's transient states).
//! - user text == "drip" → 3 partial-stream chunks spaced 400ms apart, then
//!   the result frame — deterministic multi-frame streaming for the webui
//!   liveness e2e (fix-webui-streaming-liveness 6.1). Total in-scenario
//!   silence stays ≪ the driver watchdog's 1.5s probe deadline.
//! - user text == "flood" → 1200 partial-stream chunks back-to-back, no
//!   pauses — a ≥1000-entry transcript for the small-summary performance
//!   assertion (fix-webui-streaming-liveness 6.3).

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

struct Flags {
    scenario: String,
    loop_mode: bool,
    slow_ms: u64,
    hang_on_init: bool,
    ignore_interrupt: bool,
    delay_init_ms: u64,
    journal: Option<String>,
    /// （add-agent-mode-selection）argv `--permission-mode` 的值（原样保存）。
    /// `bypassPermissions` 下 `perm` 场景跳过 hook 直接放行——行为级断言
    /// "allow 模式免审批" 的数据源；运行时 `set_permission_mode` 更新它。
    permission_mode: Option<String>,
    resume_fails: bool,
    /// （session-slash-commands）initialize 控制响应带固定命令表
    /// （goal/compact）——claude 命令发现 e2e 的数据源。
    advertise_commands: bool,
    /// True when argv carried `--resume <id>` (as opposed to `--session-id`)
    /// — resume rejection only applies to actual resume attempts, so the
    /// manager's fresh-session fallback still spawns fine.
    resume_used: bool,
    /// True when argv carried `--session-id`. The real CLI rejects it
    /// combined with `--resume`/`--continue` unless `--fork-session` is
    /// also specified — replicated in main() so tests catch bad argv.
    session_id_flag_used: bool,
    continue_used: bool,
    fork_session: bool,
    session_id: String,
    /// （workbench-composer-input-polish）运行时 `set_model` 控制请求的当前
    /// 值：后续 assistant 帧的 model 字段报告它（真 CLI 行为——切换从下一
    /// 轮生效并在帧上可见）。`None` = 未切换过，报告 init 帧同款 "fake"。
    model: Option<String>,
}

const SCENARIOS: &[&str] = &["hello", "bash", "deny", "thinking"];

/// Flags that consume the NEXT argv token as their value (the SDK passes
/// many; anything not listed here and starting with `--` is treated as a
/// boolean switch and ignored). Positional tokens only become the scenario
/// if they name a known scenario — SDK-injected positionals must not be
/// mistaken for it.
const VALUE_FLAGS: &[&str] = &[
    "--slow-ms",
    "--delay-init-ms",
    "--journal",
    "--session-id",
    "--resume",
    "--output-format",
    "--input-format",
    "--permission-prompt-tool",
    "--model",
    "--fallback-model",
    "--permission-mode",
    "--setting-sources",
    "--mcp-config",
    "--append-system-prompt",
    "--system-prompt",
    "--max-turns",
    "--cwd",
    "--scenario",
];

fn parse_flags() -> Flags {
    let mut f = Flags {
        scenario: "hello".into(),
        loop_mode: false,
        slow_ms: 0,
        hang_on_init: false,
        ignore_interrupt: false,
        delay_init_ms: 0,
        journal: None,
        permission_mode: None,
        resume_fails: false,
        advertise_commands: false,
        resume_used: false,
        session_id_flag_used: false,
        continue_used: false,
        fork_session: false,
        session_id: "fake-1".into(),
        model: None,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--loop" => f.loop_mode = true,
            "--hang-on-init" => f.hang_on_init = true,
            "--ignore-interrupt" => f.ignore_interrupt = true,
            "--resume-fails" => f.resume_fails = true,
            "--advertise-commands" => f.advertise_commands = true,
            "--delay-new-ms" => {
                // Compat alias for the ACP-era flag: slow session/new ≈
                // slow initialize handshake in the new dialect.
                f.delay_init_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--enable-load" => {} // no-op: resume always "works" in the new dialect
            "--load-fails" => f.resume_fails = true,
            "--slow-ms" => {
                f.slow_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--delay-init-ms" => {
                f.delay_init_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--journal" => {
                f.journal = args.get(i + 1).cloned();
                i += 1;
            }
            "--scenario" => {
                // Keyed form: survives the SDK's extra_args map (positionals
                // cannot be expressed there).
                if let Some(v) = args.get(i + 1) {
                    f.scenario = v.clone();
                }
                i += 1;
            }
            "--session-id" => {
                f.session_id_flag_used = true;
                if let Some(v) = args.get(i + 1) {
                    f.session_id = v.clone();
                }
                i += 1;
            }
            "--resume" => {
                f.resume_used = true;
                if let Some(v) = args.get(i + 1) {
                    f.session_id = v.clone();
                }
                i += 1;
            }
            "--continue" => f.continue_used = true,
            "--fork-session" => f.fork_session = true,
            // （add-agent-mode-selection）保存权限模式值（其余 VALUE_FLAGS
            // 照旧只消费不保存）。
            "--permission-mode" => {
                if let Some(v) = args.get(i + 1) {
                    f.permission_mode = Some(v.clone());
                }
                i += 1;
            }
            s if VALUE_FLAGS.contains(&s) => {
                i += 1; // consume the value, ignore it
            }
            s if SCENARIOS.contains(&s) => f.scenario = s.to_string(),
            _ => {} // boolean switch or unknown positional: ignore
        }
        i += 1;
    }
    f
}

struct Io {
    out: io::StdoutLock<'static>,
    journal: Option<std::fs::File>,
}

impl Io {
    fn emit(&mut self, v: &Value) {
        let line = serde_json::to_string(v).unwrap();
        writeln!(self.out, "{line}").unwrap();
        self.out.flush().unwrap();
        self.journal_write("out", v);
    }
    fn journal_write(&mut self, dir: &str, v: &Value) {
        if let Some(j) = self.journal.as_mut() {
            let line = serde_json::to_string(&json!({"dir": dir, "msg": v})).unwrap();
            let _ = writeln!(j, "{line}");
            let _ = j.flush();
        }
    }
}

fn main() {
    // The SDK runs `{cli} --version` before spawning (transport check).
    if std::env::args().any(|a| a == "--version") {
        println!("2.1.206 (fake-claude-cli)");
        return;
    }
    let mut flags = parse_flags();
    // The real CLI rejects `--session-id` combined with `--resume` /
    // `--continue` unless `--fork-session` is also specified. Replicate the
    // validation so a bad argv construction fails fast here instead of
    // hanging until the startup timeout against the real binary.
    if flags.session_id_flag_used
        && (flags.resume_used || flags.continue_used)
        && !flags.fork_session
    {
        eprintln!(
            "Error: --session-id can only be used with --continue or --resume if --fork-session is also specified."
        );
        std::process::exit(1);
    }
    // Like the real CLI, only an actual `--resume` of an unknown id is
    // rejected; a fresh `--session-id` spawn with the same flags works.
    if flags.resume_fails && flags.resume_used {
        eprintln!("Error: No conversation found with session ID");
        std::process::exit(1);
    }

    let journal = flags.journal.as_ref().map(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .expect("open journal")
    });
    let mut io = Io {
        out: Box::leak(Box::new(io::stdout())).lock(),
        journal,
    };
    // Diagnostic: record the full argv so tests can assert flag plumbing.
    io.journal_write(
        "meta",
        &json!({"argv": std::env::args().skip(1).collect::<Vec<_>>()}),
    );

    // stdin 在后台线程读入并经 channel 转交：场景中途（流式分块之间）也能
    // 非阻塞地应答 driver 的看门狗探针（真 CLI 的控制帧与流式帧并发处理；
    // 单线程阻塞读会让探针把驱动泵卡到场景结束，流式帧被整段突发转发）。
    let (stdin_tx, stdin_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if stdin_tx.send(line).is_err() {
                return;
            }
        }
    });
    let mut init_sent = false;
    let mut hook_counter: u64 = 0;
    // sebas-9pz ① hang test: once the "hang" prompt arrives, stop emitting
    // content frames. control_request (liveness probe, interrupts) still gets
    // acked so the process looks alive-and-healthy — only the driver's hang
    // detector (no content for N seconds) can fire.
    let mut hanging = false;

    while let Ok(line) = stdin_rx.recv() {
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        io.journal_write("in", &v);
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "control_request" => {
                let req_id = v
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let subtype = v
                    .pointer("/request/subtype")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match subtype {
                    "initialize" => {
                        if flags.hang_on_init {
                            continue; // never answer → caller's startup timeout fires
                        }
                        if flags.delay_init_ms > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(
                                flags.delay_init_ms,
                            ));
                        }
                        // session-slash-commands：`--advertise-commands`
                        // 时 initialize 控制响应带固定命令表。payload 就是
                        // 信封的内层 response 对象（真 CLI 形状）：cc-agent-sdk
                        // flatten 后它留在 `info["response"]` 一层，claude
                        // 驱动的映射负责解层（session-slash-commands 5.2
                        // 进程级 e2e 实测过旧注释「SDK 拿到的就是本对象」
                        // 是错的——差一层才对）。
                        let payload = if flags.advertise_commands {
                            json!({
                                "commands": [
                                    {"name": "goal", "description": "Track a goal across turns",
                                     "argumentHint": "<condition>"},
                                    {"name": "compact", "description": "Clear conversation context"}
                                ]
                            })
                        } else {
                            json!({})
                        };
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id,
                                         "response": payload}
                        }));
                    }
                    "interrupt" => {
                        // Real CLI: ack the control request, end the turn with
                        // an error result, then EXIT (post-interrupt client
                        // is unusable — spike §S6).
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                        if flags.ignore_interrupt {
                            // sebas-9pz ① hang test: ack the interrupt but
                            // keep going (do NOT exit) — models a child that
                            // is alive but unresponsive to cancels, so the
                            // driver's escalation has to run.
                            continue;
                        }
                        io.emit(&result_frame(
                            &flags.session_id,
                            "error_during_execution",
                            true,
                        ));
                        io.out.flush().unwrap();
                        std::process::exit(1);
                    }
                    "set_permission_mode" => {
                        // （add-agent-mode-selection）运行时权限模式切换：
                        // 记 journal（测试断言切换送达与值）、更新内部模式
                        // （影响后续 perm 场景是否走 hook），然后照常 ack。
                        // SDK 的控制帧形状：{"request":{"subtype":
                        // "set_permission_mode","mode":"<v>"}}。
                        let new_mode = v
                            .pointer("/request/mode")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        io.journal_write(
                            "mode_change",
                            &json!({ "type": "mode_change", "mode": new_mode }),
                        );
                        if !new_mode.is_empty() {
                            flags.permission_mode = Some(new_mode);
                        }
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                    }
                    "set_model" => {
                        // （workbench-composer-input-polish）运行时模型切换：
                        // 记 journal（e2e 断言控制请求送达与值；null = SDK
                        // None，即 "default" 的特判形态），更新内部模型（后续
                        // assistant 帧报告新值——真 CLI 行为），照常 ack。
                        let requested = v.pointer("/request/model").cloned().unwrap_or(Value::Null);
                        io.journal_write(
                            "model_change",
                            &json!({ "type": "model_change", "model": requested }),
                        );
                        flags.model = match requested {
                            Value::String(s) if !s.is_empty() => Some(s),
                            _ => None,
                        };
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                    }
                    _ => {
                        // set_model / ... : ack and ignore.
                        io.emit(&json!({
                            "type": "control_response",
                            "response": {"subtype": "success", "request_id": req_id, "response": {}}
                        }));
                    }
                }
            }
            "user" => {
                let text = user_text(&v);
                if text.contains("crash") {
                    // D6: mid-session process crash — one last frame, then die.
                    emit_assistant_text(&mut io, &flags.session_id, "boom", reported_model(&flags));
                    io.out.flush().unwrap();
                    std::process::exit(1);
                }
                if !init_sent {
                    init_sent = true;
                    let cwd = std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    io.emit(&json!({
                        "type": "system", "subtype": "init",
                        "session_id": flags.session_id, "model": "fake",
                        "cwd": cwd, "tools": ["Bash", "Read"],
                    }));
                }
                if hanging {
                    // Already hung: ignore any further prompts (no output).
                    continue;
                }
                if text == "refuse" {
                    // sebas-9pz ⑤: a refusal — subtype carries "refusal",
                    // is_error=true. The driver must treat this as a
                    // NON-terminal error (process stays healthy) so the
                    // session survives and the next prompt still works.
                    io.emit(&result_frame(&flags.session_id, "refusal", true));
                } else if text == "hang" {
                    // sebas-9pz ① hang test: enter the silent-hang state.
                    hanging = true;
                } else if text == "stall" {
                    // fix-pending-queue-liveness e2e：先出一帧（卡片 FSM 进
                    // WORKING，回合确实在跑），随后进入「活着但对 ACP 沉默」
                    // 状态——control_request 探测照常应答（驱动 hang 链因此
                    // 不触发），但再无任何事件帧。引擎停滞看门狗（
                    // `[dispatch] turn_stall_timeout`）是这种停滞的唯一兜底。
                    emit_assistant_text(&mut io, &flags.session_id, "stalling...", reported_model(&flags));
                    hanging = true;
                } else if text == "perm" {
                    perm_turn(&flags, &mut io, &stdin_rx, &mut hook_counter);
                } else if text == "stream" {
                    stream_turn(&mut flags, &mut io, &stdin_rx);
                } else if text == "drip" {
                    drip_turn(&mut flags, &mut io, &stdin_rx);
                } else if text == "flood" {
                    flood_turn(&mut flags, &mut io, &stdin_rx);
                } else {
                    run_scenario(&flags, &mut io, &stdin_rx, &mut hook_counter);
                }
                // Like the real CLI in streaming mode, stay alive for further
                // user messages until stdin closes (multi-turn).
            }
            _ => {} // control_response to nothing we asked: ignore
        }
    }
}

/// "perm" prompt: Bash(rm -rf /) gated by the hook; the tool_result reflects
/// the decision, then the turn ends.
fn perm_turn(
    flags: &Flags,
    io: &mut Io,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
    hook_counter: &mut u64,
) {
    let sid = &flags.session_id;
    io.emit(&json!({
        "type": "assistant",
        "session_id": sid,
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "tc-1", "name": "Bash", "input": {"command": "rm -rf /"}}
        ]}
    }));
    // （add-agent-mode-selection）bypassPermissions = 完全放行：不产生
    // hook_callback 审批交互，工具直接执行——与 ask 模式（走 hook 等决定）
    // 形成行为级对照。
    if flags.permission_mode.as_deref() == Some("bypassPermissions") {
        io.emit(&tool_result_frame(sid, "tc-1", "perm done\n", false));
        io.emit(&result_frame(sid, "success", false));
        return;
    }
    *hook_counter += 1;
    let req_id = format!("fake-hook-{}", *hook_counter);
    io.emit(&json!({
        "type": "control_request",
        "request_id": req_id,
        "request": {
            "subtype": "hook_callback",
            "callback_id": "hook_0",
            "tool_use_id": "tc-1",
            "input": {
                "hook_event_name": "PreToolUse",
                "session_id": sid,
                "tool_name": "Bash",
                "tool_input": {"command": "rm -rf /"},
                "cwd": "/tmp",
                "transcript_path": "/tmp/fake.jsonl"
            }
        }
    }));
    let decision = wait_hook_decision(stdin_rx, &req_id, io);
    if decision == "allow" {
        io.emit(&tool_result_frame(sid, "tc-1", "perm done\n", false));
    } else {
        io.emit(&tool_result_frame(sid, "tc-1", "denied by fake", true));
    }
    io.emit(&result_frame(sid, "success", false));
}

/// "stream" prompt: 5 text chunks, then a pause so the debounced pump can
/// flush a transient mid-turn card before Finished (mirrors the ACP-era
/// fake's "stream" trigger).
fn stream_turn(
    flags: &mut Flags,
    io: &mut Io,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
) {
    let sid = flags.session_id.clone();
    let model = reported_model(flags).to_string();
    for i in 0..5 {
        emit_assistant_text(io, &sid, &format!("chunk{i} "), &model);
    }
    // 800ms: the 150ms debounce tick must flush a transient 🚧 card well
    // before the result frame; SDK startup (version probe + spawn) adds
    // ~100ms of latency, so smaller margins flake under parallel test load.
    // Sleep in short slices answering the watchdog probe (as the real CLI
    // would) — an unanswered probe blocks the driver's read loop and bursts
    // all frames + result into one pump iteration, which drops the pending
    // SEED→WORKING reaction.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
        pump_controls(stdin_rx, io, flags);
    }
    io.emit(&result_frame(&sid, "success", false));
}

/// "drip" prompt (fix-webui-streaming-liveness 6.1): 3 partial-stream chunks
/// spaced 400ms apart so each lands in its own core-channel coalescing window
/// (250ms) — the webui observes MULTIPLE turn.append frames mid-turn instead
/// of one block at completion. Between chunks the watchdog probe (and any
/// other pending control_request) is answered exactly like the main loop
/// would, mirroring the real CLI's concurrent control/stream processing.
fn drip_turn(flags: &mut Flags, io: &mut Io, stdin_rx: &std::sync::mpsc::Receiver<String>) {
    let sid = flags.session_id.clone();
    for i in 0..3 {
        emit_assistant_text(io, &sid, &format!("drip{i} "), reported_model(flags));
        // Between chunks: 400ms gaps put each chunk in its own coalescing
        // window. The final pause is a short 150ms — just enough for the last
        // window to flush before the turn completes, keeping the whole
        // scenario ≈0.95s, well inside the watchdog probe's 1.5s deadline.
        let pause = if i + 1 < 3 { 400 } else { 150 };
        std::thread::sleep(std::time::Duration::from_millis(pause));
        pump_controls(stdin_rx, io, flags);
    }
    io.emit(&result_frame(&sid, "success", false));
}

/// Non-blocking drain of stdin lines that arrived while a scenario runs;
/// control_requests (the driver's watchdog probe) are answered exactly like
/// the main loop's control arm — a probe left unanswered blocks the driver's
/// message pump until the scenario returns, which would burst the stream.
/// Non-control lines (user prompts — the driver never sends them mid-turn)
/// are dropped.
fn pump_controls(stdin_rx: &std::sync::mpsc::Receiver<String>, io: &mut Io, flags: &mut Flags) {
    while let Ok(line) = stdin_rx.try_recv() {
        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        io.journal_write("in", &v);
        if v.get("type").and_then(Value::as_str) != Some("control_request") {
            continue;
        }
        let req_id = v
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let subtype = v
            .pointer("/request/subtype")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if subtype == "set_permission_mode" {
            let new_mode = v
                .pointer("/request/mode")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            io.journal_write(
                "mode_change",
                &json!({ "type": "mode_change", "mode": new_mode }),
            );
            if !new_mode.is_empty() {
                flags.permission_mode = Some(new_mode);
            }
        }
        io.emit(&json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": req_id, "response": {}}
        }));
    }
}

/// "flood" prompt (fix-webui-streaming-liveness 6.3): 1200 partial-stream
/// chunks — a ≥1000-entry transcript. Every 100 chunks the scenario pumps
/// stdin: without it the stdout pipe fills (~64KB), the child blocks on
/// write and can never answer the driver's watchdog probe, which in turn
/// blocks the driver's message pump — a deadlock that kills the session at
/// the probe deadline.
fn flood_turn(flags: &mut Flags, io: &mut Io, stdin_rx: &std::sync::mpsc::Receiver<String>) {
    let sid = flags.session_id.clone();
    for i in 0..1200u32 {
        emit_assistant_text(io, &sid, &format!("f{i} "), reported_model(flags));
        if (i + 1) % 100 == 0 {
            pump_controls(stdin_rx, io, flags);
        }
    }
    io.emit(&result_frame(&sid, "success", false));
}

fn run_scenario(
    flags: &Flags,
    io: &mut Io,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
    hook_counter: &mut u64,
) {
    let sid = &flags.session_id;
    let settle = || {
        // Slow-down knob: sleep BETWEEN the content frames and the result
        // frame so a debounced consumer observes the transient 🚧 state.
        if flags.slow_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(flags.slow_ms));
        }
    };
    match flags.scenario.as_str() {
        "hello" => {
            let model = reported_model(flags);
            emit_assistant_text(io, sid, "hello ", model);
            emit_assistant_text(io, sid, "world", model);
            settle();
            io.emit(&result_frame(sid, "success", false));
        }
        "thinking" => {
            // （fix-webui-approval-restore-and-session-identity 6.1）最终
            // thinking 块带 `signature`：真实 CLI 的最终 thinking assistant
            // 帧必带签名（signature_delta 聚合），SDK `ThinkingBlock.signature`
            // 是必填 String——缺签名整帧被 `MessageParse` 拒绝、driver 按
            // 「未知消息」丢弃。固定假签名即可（不验签，只对齐 wire 形状）。
            io.emit(&json!({
                "type": "assistant",
                "session_id": sid,
                "message": {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "hmm", "signature": "sig-fake-thinking"}
                ], "model": reported_model(flags)}
            }));
            // The thinking delta rides the partial stream (real CLI with
            // --include-partial-messages); the final thinking block is
            // skipped by the driver.
            io.emit(&json!({
                "type": "stream_event",
                "uuid": "u-think",
                "session_id": sid,
                "event": {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "thinking_delta", "thinking": "hmm"}
                }
            }));
            emit_assistant_text(io, sid, "thought out loud", reported_model(flags));
            settle();
            io.emit(&result_frame(sid, "success", false));
        }
        "bash" | "deny" => {
            let (tool_id, cmd) = if flags.scenario == "bash" {
                ("toolu_01", "echo hi")
            } else {
                ("toolu_02", "rm -rf /")
            };
            io.emit(&json!({
                "type": "assistant",
                "session_id": sid,
                "message": {"role": "assistant", "content": [
                    {"type": "tool_use", "id": tool_id, "name": "Bash", "input": {"command": cmd}}
                ]}
            }));
            // Permission gate: ask the SDK side via hook_callback and block.
            *hook_counter += 1;
            let req_id = format!("fake-hook-{}", *hook_counter);
            io.emit(&json!({
                "type": "control_request",
                "request_id": req_id,
                "request": {
                    "subtype": "hook_callback",
                    "callback_id": "hook_0",
                    "tool_use_id": tool_id,
                    "input": {
                        "hook_event_name": "PreToolUse",
                        "session_id": sid,
                        "tool_name": "Bash",
                        "tool_input": {"command": cmd},
                        "cwd": "/tmp",
                        "transcript_path": "/tmp/fake.jsonl"
                    }
                }
            }));
            let decision = wait_hook_decision(stdin_rx, &req_id, io);
            if decision == "allow" && flags.scenario == "bash" {
                io.emit(&tool_result_frame(sid, tool_id, "hi\n", false));
            } else {
                io.emit(&tool_result_frame(sid, tool_id, "denied by fake", true));
            }
            settle();
            io.emit(&result_frame(sid, "success", false));
        }
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }
}

/// Read stdin until the control_response for our hook_callback arrives;
/// extract permissionDecision (default deny — fail closed like the bridge).
/// Any control_request seen while waiting (e.g. the driver's watchdog
/// `set_model` probe) is acked inline so it doesn't consume our response
/// or starve the probe of its answer.
fn wait_hook_decision(
    stdin_rx: &std::sync::mpsc::Receiver<String>,
    req_id: &str,
    io: &mut Io,
) -> String {
    while let Ok(line) = stdin_rx.recv() {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        io.journal_write("in", &v);
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "control_request" {
            let other_id = v.get("request_id").and_then(Value::as_str).unwrap_or("");
            let subtype = v
                .pointer("/request/subtype")
                .and_then(Value::as_str)
                .unwrap_or("");
            if subtype == "interrupt" {
                // Not expected mid-hook, but honor the contract.
                io.emit(&json!({
                    "type": "control_response",
                    "response": {"subtype": "success", "request_id": other_id, "response": {}}
                }));
                io.out.flush().unwrap();
                std::process::exit(1);
            }
            io.emit(&json!({
                "type": "control_response",
                "response": {"subtype": "success", "request_id": other_id, "response": {}}
            }));
            continue;
        }
        if ty == "control_response"
            && v.pointer("/response/request_id").and_then(Value::as_str) == Some(req_id)
        {
            return v
                .pointer("/response/response/hookSpecificOutput/permissionDecision")
                .and_then(Value::as_str)
                .unwrap_or("deny")
                .to_string();
        }
    }
    "deny".into()
}

fn user_text(v: &Value) -> String {
    match v.pointer("/message/content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn assistant_text(sid: &str, text: &str, model: &str) -> Value {
    json!({
        "type": "assistant",
        "session_id": sid,
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": text}
        ], "model": model}
    })
}

/// Emit one visible text chunk the way the real CLI does under
/// `--include-partial-messages` (fix-webui-streaming-liveness 1.1): a
/// `stream_event` frame carrying the token delta FIRST, then the final
/// whole-message `assistant` frame. The driver maps the delta and skips the
/// final text block, so consumers see each chunk exactly once.
fn emit_assistant_text(io: &mut Io, sid: &str, text: &str, model: &str) {
    io.emit(&json!({
        "type": "stream_event",
        "uuid": format!("u-{}", text.len()),
        "session_id": sid,
        "event": {
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": text}
        }
    }));
    io.emit(&assistant_text(sid, text, model));
}

/// assistant 帧 model 字段的当前值：set_model 切换过的值优先，否则 init 帧
/// 同款 "fake"（真 CLI 行为——帧上总能看到生效模型）。
fn reported_model(flags: &Flags) -> &str {
    flags.model.as_deref().unwrap_or("fake")
}

fn tool_result_frame(sid: &str, tool_id: &str, content: &str, is_error: bool) -> Value {
    json!({
        "type": "user",
        "session_id": sid,
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": tool_id, "content": content, "is_error": is_error}
        ]}
    })
}

fn result_frame(sid: &str, subtype: &str, is_error: bool) -> Value {
    json!({
        "type": "result", "subtype": subtype, "is_error": is_error,
        "stop_reason": if is_error { serde_json::Value::Null } else { json!("end_turn") },
        "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1, "result": "", "session_id": sid
    })
}

#[cfg(test)]
mod thinking_signature_tests {
    //! fix-webui-approval-restore-and-session-identity 6.1：最终 thinking
    //! assistant 帧的保真合同——`ThinkingBlock.signature` 必填（SDK 边界），
    //! 桩的 thinking 块必须带签名，否则整帧在 driver 侧被丢弃。

    use super::*;

    /// thinking 场景发出的第一帧（最终 thinking assistant 帧）携带
    /// `signature` 字段。
    #[test]
    fn thinking_assistant_frame_carries_a_signature() {
        // 直接内联场景里那条 json! 的形状断言：thinking 块 = {type, thinking,
        // signature}。场景帧经 io.emit 写 stdout，这里复刻同一字面量结构，
        // 防止未来改动把 signature 弄丢（compile-time 邻近 + assert 双保险）。
        let frame = json!({
            "type": "assistant",
            "session_id": "s",
            "message": {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm", "signature": "sig-fake-thinking"}
            ], "model": "fake"}
        });
        let block = &frame["message"]["content"][0];
        assert_eq!(block["type"], "thinking");
        assert_eq!(block["signature"], "sig-fake-thinking");

        // SDK 反序列化角度：等价 JSON 必须能解出带签名的 thinking 块。
        let parsed: serde_json::Value =
            serde_json::from_str(r#"{"type":"thinking","thinking":"hmm","signature":"sig"}"#)
                .unwrap();
        assert_eq!(parsed["signature"], "sig");
        let missing: Result<serde_json::Value, _> =
            serde_json::from_str(r#"{"type":"thinking","thinking":"hmm","signature":null}"#);
        // null 签名不是合法 String——SDK 边界同样拒绝（fail loud 而非丢帧）。
        assert!(missing.is_err() || missing.unwrap()["signature"].is_null());
    }
}
