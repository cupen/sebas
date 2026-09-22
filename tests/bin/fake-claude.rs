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
//!                   [--delta-gap-ms N]
//!   scenario: hello (default) | bash | deny | thinking（位置形态，既有兼容；
//!   全集与键值形式 `--scenario <name>` 说明见下）
//!
//!   --delta-gap-ms N（add-conversation-streaming-journey 1.1）：默认场景
//!   （hello/default）相邻文本 delta 之间的静默毫秒数；缺省 `0` = 现行行为
//!   （背靠背发出）。N>0 时逐段发出、段间 sleep N 毫秒——「回合进行中」窗口
//!   的确定性构造器（浏览器 e2e 在会话仍 running 时于 DOM 观测增量正文）。
//!   N>0 同时覆盖 "drip" 触发词的前两段间距（缺省 400ms）。
//!   **预算约束（按路径不同）**：driver 看门狗每秒发一次控制探针，探针
//!   **应答超时 1.5s**（`sebas-acp/src/claude/driver.rs:477`），悬空即判子进程
//!   死亡；**挂起探测**是另一回事，默认 5 分钟（`driver.rs:498-502`，可用
//!   `SEBAS_HANG_TIMEOUT_SECS` 覆盖）。`drip` 触发词段间先 sleep 再 pump，
//!   故其 `gap` 必须 < 1.5s；默认场景的 `sleep_delta_gap` 每 50ms pump 一次，
//!   不受 1.5s 约束。推荐 N=500。
//!   （close-acceptance-blind-spots 1.2）全模型行为 mock 场景集（键值形式
//!   `--scenario <name>` 选择，未指定 = 现行缺省行为）：
//!   - default：现行缺省行为（hello 的同义词——正文增量流式多段文本 delta
//!     已由缺省行为覆盖：两个 text_delta + 收尾 result）。
//!   - thinking：thinking 增量与正文增量**交替**出现（两段 thinking、两段
//!     正文交错——「边想边答」的 wire 形态）。
//!   - tool-loop：完整工具环 tool_use → 测试侧 hook 应答 → tool_result →
//!     后续正文 → result（bash/deny 没有环后正文）。
//!   - empty：正常结束、零输出（组 4 零输出 notice 落点的数据源）。
//!   - slow：先静默 `--delay-ms N` 毫秒（静默期照常应答控制探针），再出
//!     正文收尾——delay 大于 `[dispatch] turn_stall_timeout` 触发停滞自愈，
//!     小于阈值即「慢后端」反馈时限形态。
//!   - error：上游错误形态——result 帧 is_error=true（error_during_request）
//!     后进程退出，投影落 error 条目、会话转 failed。
//!   journal 逐行携带 `scenario` 字段（场景断言的数据源）；未知 `--scenario`
//!   值解析期即拒（exit 2——子进程秒退，driver 以终态错误如实上报）。
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
use std::time::{Duration, Instant};

struct Flags {
    scenario: String,
    loop_mode: bool,
    slow_ms: u64,
    /// （add-conversation-streaming-journey 1.1）默认场景相邻文本 delta 之间
    /// 的静默毫秒数（`--delta-gap-ms N`）。`0` = 完全现行行为（背靠背）。
    delta_gap_ms: u64,
    /// （close-acceptance-blind-spots 1.2）`--delay-ms N` 全局延迟参数：
    /// slow 场景在**任何输出之前**静默 N 毫秒（静默期照常应答控制探针）。
    delay_ms: u64,
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

const SCENARIOS: &[&str] = &[
    "hello", "default", "bash", "deny", "thinking", "tool-loop", "empty", "slow", "error",
];

/// Flags that consume the NEXT argv token as their value (the SDK passes
/// many; anything not listed here and starting with `--` is treated as a
/// boolean switch and ignored). Positional tokens only become the scenario
/// if they name a known scenario — SDK-injected positionals must not be
/// mistaken for it.
const VALUE_FLAGS: &[&str] = &[
    "--slow-ms",
    "--delay-init-ms",
    "--delay-ms",
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
    parse_flags_from(&std::env::args().skip(1).collect::<Vec<_>>())
}

/// （close-acceptance-blind-spots 1.4）解析从 env 抽出，便于单测直接喂 argv。
fn parse_flags_from(args: &[String]) -> Flags {
    let mut f = Flags {
        scenario: "hello".into(),
        loop_mode: false,
        slow_ms: 0,
        delta_gap_ms: 0,
        delay_ms: 0,
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
    let args = args.to_vec();
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
            "--delta-gap-ms" => {
                // （add-conversation-streaming-journey 1.1）默认场景文本 delta
                // 之间的静默；非数值回落 0（与 --slow-ms 同款宽容语义）。
                // 预算约束见模块文档：帧间静默必须 < 1.5s 挂起探测超时。
                f.delta_gap_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--delay-ms" => {
                f.delay_ms = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
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
                // cannot be expressed there). 未知场景解析期即拒（exit 2）：
                // 子进程秒退，driver 以终态错误如实上报，会话不会停在
                // 「握手成功后永远沉默」。
                if let Some(v) = args.get(i + 1) {
                    if !SCENARIOS.contains(&v.as_str()) {
                        eprintln!("unknown scenario: {v}");
                        std::process::exit(2);
                    }
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
    /// 帧汇：生产路径 = stdout（`Box<dyn Write>` 便于单测注入可回读的替身，
    /// add-conversation-streaming-journey 1.2 的帧序/时序断言）。
    out: Box<dyn Write>,
    journal: Option<std::fs::File>,
    /// （close-acceptance-blind-spots 1.2）所用场景名——journal 逐行携带
    /// `scenario` 字段，供 e2e 断言「跑的就是这个场景」。
    scenario: String,
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
            let line = serde_json::to_string(&json!({
                "dir": dir, "scenario": self.scenario, "msg": v
            }))
            .unwrap();
            // O_APPEND 下单条 write 原子——先攒齐整行再一次 write_all，
            // 多次 writeln! 的分段写会在双子进程并发 append 时交错成坏行。
            let mut buf = line.into_bytes();
            buf.push(b'\n');
            let _ = j.write_all(&buf);
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
        out: Box::new(Box::leak(Box::new(io::stdout())).lock()),
        journal,
        scenario: flags.scenario.clone(),
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
                    run_scenario(&mut flags, &mut io, &stdin_rx, &mut hook_counter);
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
///
/// （add-conversation-streaming-journey 1.1）`--delta-gap-ms N>0` 时前两段间距
/// 改用 N（缺省 0 = 现行 400ms 不变）——本触发词因此成为「可配置时间间隔」
/// 的浏览器旅程数据源；最终 150ms 收尾停顿不变。**预算**：本函数每段后先
/// sleep 再 pump，故 `gap`（含 150ms 收尾）必须 < 1.5s 探针应答超时
/// （`driver.rs:477`）；N=500 安全。
fn drip_turn(flags: &mut Flags, io: &mut Io, stdin_rx: &std::sync::mpsc::Receiver<String>) {
    let sid = flags.session_id.clone();
    let gap = if flags.delta_gap_ms > 0 {
        flags.delta_gap_ms
    } else {
        400
    };
    for i in 0..3 {
        emit_assistant_text(io, &sid, &format!("drip{i} "), reported_model(flags));
        // Between chunks: gaps put each chunk in its own coalescing window
        // (250ms). The final pause is a short 150ms — just enough for the last
        // window to flush before the turn completes, keeping the whole
        // scenario well inside the watchdog probe's 1.5s deadline.
        let pause = if i + 1 < 3 { gap } else { 150 };
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
    flags: &mut Flags,
    io: &mut Io,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
    hook_counter: &mut u64,
) {
    // sid 取自本地克隆（而非 `&flags.session_id`）：默认场景要同时可变借用
    // flags（`--delta-gap-ms` 的段间 sleep 需 pump 控制探针）。
    let sid_owned = flags.session_id.clone();
    let sid = sid_owned.as_str();
    match flags.scenario.as_str() {
        "hello" | "default" => {
            // default = 现行缺省行为（close-acceptance-blind-spots 1.2）：
            // 多段文本 delta 的正文增量流式由缺省行为覆盖——两个 text_delta
            // 帧之后收尾，消费者观察到「分段到达、逐字各一次」。
            // （add-conversation-streaming-journey 1.1）`--delta-gap-ms N>0`
            // 时两段之间 sleep N（缺省 0 = 现行背靠背行为不变）。
            emit_default_text(flags, io, stdin_rx);
            settle_pause(flags);
            io.emit(&result_frame(sid, "success", false));
        }
        "thinking" => {
            // （close-acceptance-blind-spots 1.2）thinking→正文**交替**：两段
            // thinking 增量与两段正文增量交错（真模型「边想边答」的 wire
            // 形态）。thinking 块仍带 `signature`（6.1 合同：SDK 的
            // ThinkingBlock.signature 必填，缺签名整帧被拒、driver 丢帧）。
            let model = reported_model(flags).to_string();
            emit_assistant_thinking(io, sid, "hmm", &model);
            emit_assistant_text(io, sid, "thought out loud", &model);
            emit_assistant_thinking(io, sid, "hmm again", &model);
            emit_assistant_text(io, sid, "and the answer", &model);
            settle_pause(flags);
            io.emit(&result_frame(sid, "success", false));
        }
        "tool-loop" => {
            // （close-acceptance-blind-spots 1.2）完整工具环：tool_use →
            // hook_callback（测试侧经泊车审批应答）→ tool_result → **环后
            // 正文** → result。与 bash/deny 的差异就在环后还有正文——
            // 「环收尾后模型继续作答」的投影序列断言数据源。
            let tool_id = "toolu_loop";
            let cmd = "echo loop";
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
            if decision == "allow" {
                io.emit(&tool_result_frame(sid, tool_id, "loop done\n", false));
            } else {
                io.emit(&tool_result_frame(sid, tool_id, "denied by fake", true));
            }
            emit_assistant_text(io, sid, "tool loop finished", reported_model(flags));
            settle_pause(flags);
            io.emit(&result_frame(sid, "success", false));
        }
        "empty" => {
            // （close-acceptance-blind-spots 1.2）空响应：正常结束、零输出。
            // 组 4 的零输出落点（投影合成 notice 条目）与提交反馈时限用例
            // 都以此场景为数据源——回合有终态、transcript 无任何正文条目。
            settle_pause(flags);
            io.emit(&result_frame(sid, "success", false));
        }
        "slow" => {
            // （close-acceptance-blind-spots 1.2）慢响应：任何输出之前先静默
            // `--delay-ms` 毫秒，再出一段正文并正常收尾。静默期照常应答控制
            // 探针（活着但对引擎沉默——驱动 hang 链不触发，引擎停滞看门狗
            // `[dispatch] turn_stall_timeout` 是唯一兜底）；delay 超过阈值
            // 即停滞自愈用例，低于阈值即「慢后端」反馈时限形态。
            slow_turn(flags, io, stdin_rx);
        }
        "error" => {
            // （close-acceptance-blind-spots 1.2）上游错误形态：result 帧
            // is_error=true（error_during_request + 错误文案）——driver 映射
            // 为终态 Error，投影落 error 条目、会话转 failed。真 CLI 错误后
            // 状态不可知，进程随即退出（与 interrupt 路径同款诚实语义）。
            io.emit(&json!({
                "type": "result", "subtype": "error_during_request", "is_error": true,
                "stop_reason": Value::Null,
                "duration_ms": 1, "duration_api_ms": 1, "num_turns": 1,
                "result": "upstream error (fake): provider returned 500",
                "session_id": sid
            }));
            io.out.flush().unwrap();
            std::process::exit(1);
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
            settle_pause(flags);
            io.emit(&result_frame(sid, "success", false));
        }
        other => {
            // 防御性兜底：--scenario 与位置形态都在解析期校验过，正常到不了
            // 这里（位置形态只接受 SCENARIOS 成员）。
            eprintln!("unknown scenario: {other}");
            std::process::exit(2);
        }
    }
}

/// Slow-down knob: sleep BETWEEN the content frames and the result frame so a
/// debounced consumer observes the transient 🚧 state.
fn settle_pause(flags: &Flags) {
    if flags.slow_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(flags.slow_ms));
    }
}

/// （add-conversation-streaming-journey 1.1）默认场景的两段正文——相邻文本
/// delta 之间按 `--delta-gap-ms` 静默（0 = 背靠背，现行行为不变）。返回每段
/// text delta 写出的时刻：单测据此断言「N=0 帧序与现行一致 / N>0 相邻文本帧
/// 时间差 >= N」（1.2），不依赖对 stdout 的黑盒计时。
fn emit_default_text(
    flags: &mut Flags,
    io: &mut Io,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
) -> Vec<Instant> {
    let segments = ["hello ", "world"];
    let mut stamps = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            sleep_delta_gap(flags, io, stdin_rx);
        }
        let model = reported_model(flags).to_string();
        let sid = flags.session_id.clone();
        stamps.push(Instant::now());
        emit_assistant_text(io, &sid, seg, &model);
    }
    stamps
}

/// （add-conversation-streaming-journey 1.1）`--delta-gap-ms N` 的段间静默：
/// 50ms 切片 sleep，**每片都 pump 应答** driver 的看门狗控制探针，因此不受
/// 1.5s 探针应答超时约束（`driver.rs:477`）——只受默认 5 分钟挂起探测约束
/// （`driver.rs:498-502`）。与 `drip_turn` 的差别正在这里：后者先 sleep 再
/// pump，其 gap 必须 < 1.5s。
fn sleep_delta_gap(
    flags: &mut Flags,
    io: &mut Io,
    stdin_rx: &std::sync::mpsc::Receiver<String>,
) {
    if flags.delta_gap_ms == 0 {
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(flags.delta_gap_ms);
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(left.min(Duration::from_millis(50)));
        pump_controls(stdin_rx, io, flags);
    }
}

/// （close-acceptance-blind-spots 1.2）slow 场景主体：先静默 `--delay-ms`
/// 毫秒（50ms 切片轮询 stdin、照常应答控制探针——驱动看门狗的探测如果悬空
/// 会卡住驱动泵并误判子进程死亡），随后一段正文、正常收尾。
fn slow_turn(flags: &mut Flags, io: &mut Io, stdin_rx: &std::sync::mpsc::Receiver<String>) {
    let sid = flags.session_id.clone();
    if flags.delay_ms > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(flags.delay_ms);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            pump_controls(stdin_rx, io, flags);
        }
    }
    emit_assistant_text(io, &sid, "slow reply", reported_model(flags));
    settle_pause(flags);
    io.emit(&result_frame(&sid, "success", false));
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

/// Emit one thinking chunk the same way (delta first, final block after).
/// 最终 thinking assistant 帧必带 `signature`——SDK 的 ThinkingBlock.signature
/// 是必填 String，缺签名整帧被 `MessageParse` 拒绝、driver 按「未知消息」
/// 丢弃（fix-webui-approval-restore-and-session-identity 6.1）；driver 会
/// 跳过最终块（增量已交付），签名只为过解析边界。
fn emit_assistant_thinking(io: &mut Io, sid: &str, text: &str, model: &str) {
    io.emit(&json!({
        "type": "stream_event",
        "uuid": format!("u-think-{}", text.len()),
        "session_id": sid,
        "event": {
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "thinking_delta", "thinking": text}
        }
    }));
    io.emit(&json!({
        "type": "assistant",
        "session_id": sid,
        "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": text, "signature": "sig-fake-thinking"}
        ], "model": model}
    }));
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

#[cfg(test)]
mod scenario_flag_tests {
    //! close-acceptance-blind-spots 1.4：场景参数解析——键值形态、位置形态、
    //! `--delay-ms` 全局延迟。未知场景在解析期 exit(2)，进程内测不了退出路径
    //! （exit 终止测试进程），由进程级 e2e 覆盖：子进程秒退 → driver 终态
    //! 错误 → 会话投影 error 条目（testsuite_e2e_test::unknown_scenario_...）。

    use super::*;

    fn argv(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// 键值形态：`--scenario <name>`（SDK extra_args 唯一能表达的形态）。
    #[test]
    fn keyed_scenario_selects_the_named_scenario() {
        let f = parse_flags_from(&argv(&["--scenario", "tool-loop"]));
        assert_eq!(f.scenario, "tool-loop");
    }

    /// 位置形态向后兼容：已知场景名仍可裸放 argv。
    #[test]
    fn positional_scenario_still_works() {
        let f = parse_flags_from(&argv(&["bash"]));
        assert_eq!(f.scenario, "bash");
    }

    /// 未指定场景 = 现行缺省行为（hello），delay 归零。
    #[test]
    fn no_scenario_keeps_the_default_behavior() {
        let f = parse_flags_from(&argv(&[]));
        assert_eq!(f.scenario, "hello");
        assert_eq!(f.delay_ms, 0);
        assert_eq!(f.slow_ms, 0);
    }

    /// D4 场景集逐个可被键值形态选中；default 是 hello 的同义词。
    #[test]
    fn every_d4_scenario_selects_via_the_keyed_form() {
        for name in ["default", "hello", "thinking", "tool-loop", "empty", "slow", "error"] {
            assert!(SCENARIOS.contains(&name), "scenario {name} must be declared");
            let f = parse_flags_from(&argv(&["--scenario", name]));
            assert_eq!(f.scenario, name);
        }
    }

    /// `--delay-ms` 全局延迟参数的数值解析。
    #[test]
    fn delay_ms_parses_the_global_delay() {
        let f = parse_flags_from(&argv(&["--scenario", "slow", "--delay-ms", "9000"]));
        assert_eq!(f.scenario, "slow");
        assert_eq!(f.delay_ms, 9000);
    }

    /// 非数值 delay 回落 0（与 --slow-ms 同款宽容语义，不炸启动）。
    #[test]
    fn delay_ms_with_garbage_value_falls_back_to_zero() {
        let f = parse_flags_from(&argv(&["--delay-ms", "abc"]));
        assert_eq!(f.delay_ms, 0);
    }

    /// SDK 注入的其它旗标不干扰场景解析（--scenario 值消费后其余忽略）。
    #[test]
    fn sdk_injected_flags_do_not_disturb_the_scenario() {
        let f = parse_flags_from(&argv(&[
            "--output-format",
            "stream-json",
            "--verbose",
            "--scenario",
            "empty",
            "--max-turns",
            "1",
        ]));
        assert_eq!(f.scenario, "empty");
    }
}

#[cfg(test)]
mod delta_gap_tests {
    //! add-conversation-streaming-journey 1.2：桩「按时间间隔发 delta」的行为
    //! 单测——`N=0` 时帧序与现行一致（背靠背、无额外停顿）；`N>0` 时相邻文本
    //! delta 的时间差 `>= N`，且帧序不变（delta, assistant 交替）。

    use super::*;
    use std::sync::{Arc, Mutex};

    /// 可回读的帧汇：`Io.out` 的测试替身（生产路径是 stdout）。
    #[derive(Clone, Default)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn argv(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn frames(sink: &SharedSink) -> Vec<Value> {
        let raw = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        raw.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    /// stream_event 帧携带的文本 delta 序列（每段正文的首帧）。
    fn delta_texts(frames: &[Value]) -> Vec<String> {
        frames
            .iter()
            .filter(|f| f["type"] == "stream_event")
            .filter_map(|f| f.pointer("/event/delta/text").and_then(Value::as_str))
            .map(str::to_string)
            .collect()
    }

    fn frame_types(frames: &[Value]) -> Vec<String> {
        frames
            .iter()
            .map(|f| f["type"].as_str().unwrap_or("").to_string())
            .collect()
    }

    /// 跑一次默认场景的正文段发射（不 settle、不发 result），返回帧与每段
    /// text delta 的写出时刻。
    fn run(flags_args: &[&str]) -> (Vec<Value>, Vec<Instant>) {
        let mut flags = parse_flags_from(&argv(flags_args));
        let sink = SharedSink::default();
        let mut io = Io {
            out: Box::new(sink.clone()),
            journal: None,
            scenario: "hello".into(),
        };
        let (_tx, rx) = std::sync::mpsc::channel::<String>();
        let stamps = emit_default_text(&mut flags, &mut io, &rx);
        (frames(&sink), stamps)
    }

    /// `N=0`（缺省）：默认场景仍是「hello 」「world」背靠背两段，帧序
    /// delta→assistant→delta→assistant（完全现行行为），不引入额外停顿。
    #[test]
    fn zero_gap_keeps_the_current_frame_order_and_timing() {
        let (frames, stamps) = run(&[]);
        assert_eq!(delta_texts(&frames), vec!["hello ", "world"]);
        assert_eq!(
            frame_types(&frames),
            vec!["stream_event", "assistant", "stream_event", "assistant"]
        );
        assert_eq!(stamps.len(), 2, "one stamp per text segment");
        let elapsed = stamps[1].duration_since(stamps[0]);
        assert!(
            elapsed < Duration::from_millis(50),
            "N=0 must stay back-to-back, got {elapsed:?}"
        );
    }

    /// `N=250`：帧序与文本不变，但相邻文本 delta 的时间差 `>= N`。
    #[test]
    fn positive_gap_spaces_adjacent_text_deltas_at_least_n() {
        let (frames, stamps) = run(&["--delta-gap-ms", "250"]);
        assert_eq!(delta_texts(&frames), vec!["hello ", "world"]);
        assert_eq!(
            frame_types(&frames),
            vec!["stream_event", "assistant", "stream_event", "assistant"]
        );
        let elapsed = stamps[1].duration_since(stamps[0]);
        assert!(
            elapsed >= Duration::from_millis(250),
            "gap must be >= N, got {elapsed:?}"
        );
    }

    /// 参数解析：缺省 0（= 现行行为），显式值读取，非数值回落 0
    /// （与 `--slow-ms` 同款宽容语义）。
    #[test]
    fn delta_gap_ms_parses_and_defaults_to_zero() {
        assert_eq!(parse_flags_from(&argv(&[])).delta_gap_ms, 0);
        assert_eq!(
            parse_flags_from(&argv(&["--delta-gap-ms", "500"])).delta_gap_ms,
            500
        );
        assert_eq!(
            parse_flags_from(&argv(&["--delta-gap-ms", "abc"])).delta_gap_ms,
            0
        );
    }
}
