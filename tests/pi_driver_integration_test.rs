//! pi driver 的会话级集成旅程（add-pi-driver tasks 6.1/6.2，spec `pi-agent`）。
//!
//! 走真实生产 seam —— `SessionManager` 注册 PiDriver、`create_session` spawn
//! `pi --mode rpc` 子进程、`send` 下发命令、事件经 `event_rx` 流出 —— 用
//! **fake pi 桩**（POSIX sh 脚本，讲握手 + 剧本回合）替代真实 pi 二进制，
//! 无需 Node/pi 在场即可确定性复现。
//!
//! 覆盖：建会话握手（会话 id + 模型面 + 命令面板）→ 回合流（thinking/text/
//! tool/usage 与唯一 Finished）→ 恢复（`--session` 重挂）→ 恢复被拒诚实回落
//! → 切模型（成功 ModelChanged / 失败非终态 Error）→ 取消（abort → settled
//! → Finished）→ **看门狗 SetMode 探针被容忍**（6.2：pi 无权限系统，非终态
//! 「不支持」、会话存活）。
//!
//! 真实 pi 二进制的端到端旅程（需 Node ≥22.19 + `pi`）不在本文件：它对环境
//! 有硬依赖，放在 skip-if-absent 的进程级 e2e 里（testsuite_e2e 的 pi 段）。
//! 本文件用桩把**协议语义**钉死，桩本身即协议 fixture。

use sebas_acp::claude::manager::{AgentEntry, SessionManager};
use sebas_acp::{AcpCommand, AcpEvent, PiDriver};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// fake pi 桩：握手三连 + 剧本回合。场景经 env 注入
/// （FAKE_PI_SCENARIO / FAKE_PI_JOURNAL / FAKE_PI_SESSION）。
const FAKE_PI: &str = r#"#!/bin/sh
journal() { if [ -n "$FAKE_PI_JOURNAL" ]; then printf '%s\n' "$1" >>"$FAKE_PI_JOURNAL"; fi; }
rid() { printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'; }
journal "argv: $*"
while IFS= read -r line; do
  journal "$line"
  id=$(rid "$line")
  case "$line" in
    *'"type":"get_state"'*)
      printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"%s","model":{"id":"m-1","provider":"anthropic"}}}\n' "$id" "$FAKE_PI_SESSION" ;;
    *'"type":"get_available_models"'*)
      printf '{"id":"%s","type":"response","command":"get_available_models","success":true,"data":{"models":[{"id":"m-1","provider":"anthropic"},{"id":"g-5","provider":"openai"}]}}\n' "$id" ;;
    *'"type":"get_commands"'*)
      printf '{"id":"%s","type":"response","command":"get_commands","success":true,"data":{"commands":[{"name":"fix-tests","description":"Fix failing tests"}]}}\n' "$id" ;;
    *'"type":"prompt"'*)
      printf '{"id":"%s","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}\n' "$id"
      printf '{"type":"message_update","usage":{"input":10,"output":1},"assistantMessageEvent":{"type":"thinking_delta","delta":"hmm"}}\n'
      printf '{"type":"message_update","usage":{"input":10,"output":2},"assistantMessageEvent":{"type":"text_delta","delta":"hello world"}}\n'
      printf '{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"ls"}}\n'
      printf '{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[{"type":"text","text":"done"}]},"isError":false}\n'
      printf '{"type":"agent_end","messages":[],"willRetry":false}\n'
      printf '{"type":"agent_settled","aborted":false}\n' ;;
    *'"type":"abort"'*)
      printf '{"type":"agent_settled","aborted":true}\n'
      printf '{"id":"%s","type":"response","command":"abort","success":true}\n' "$id" ;;
    *'"type":"set_model"'*)
      case "$line" in
        *'"modelId":"bad"'*) printf '{"id":"%s","type":"response","command":"set_model","success":false,"error":"Model not found: bad"}\n' "$id" ;;
        *) printf '{"id":"%s","type":"response","command":"set_model","success":true,"data":{"id":"m-2","provider":"anthropic"}}\n' "$id" ;;
      esac ;;
  esac
done
"#;

/// 共享桩目录：每进程只写一次脚本（并发 exec 写-执行竞态见 driver 单测注释）。
fn shared_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("sebas-pi-itest-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    })
    .clone()
}

fn write_script(name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = shared_dir().join(name);
    if !path.exists() {
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path.to_string_lossy().into_owned()
}

fn pi_manager() -> Arc<SessionManager> {
    let mut agents = HashMap::new();
    agents.insert(
        "pi".to_string(),
        AgentEntry {
            driver: Arc::new(PiDriver::with_sessions_dir(Some(
                shared_dir().join("sessions").to_string_lossy().into_owned(),
            ))),
            startup_timeout: Duration::from_secs(10),
        },
    );
    Arc::new(SessionManager::new("pi".to_string(), agents))
}

/// 每测试独立的 journal 路径（并发跑时不得互相覆盖）。
fn journal_path(tag: &str) -> String {
    shared_dir()
        .join(format!("journal-{tag}.log"))
        .to_string_lossy()
        .into_owned()
}

fn drain_journal(tag: &str) {
    let _ = std::fs::remove_file(journal_path(tag));
}

fn journal_lines(tag: &str) -> Vec<String> {
    std::fs::read_to_string(journal_path(tag))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// 建一个 pi 会话（经生产 `create_session`），返回 routing id 与事件接收器。
async fn spawn_pi(
    mgr: &Arc<SessionManager>,
    exe: &str,
    scenario: &str,
    pi_session: &str,
    tag: &str,
) -> (
    String,
    Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<AcpEvent>>>,
) {
    let sid = mgr
        .create_session(
            "pi",
            vec![exe.to_string()],
            None,
            vec![
                ("FAKE_PI_SCENARIO".into(), scenario.into()),
                ("FAKE_PI_JOURNAL".into(), journal_path(tag)),
                ("FAKE_PI_SESSION".into(), pi_session.into()),
            ],
            "hi".to_string(),
        )
        .await
        .expect("pi session spawns");
    let rx = mgr.event_rx(&sid).await.expect("event rx present");
    (sid, rx)
}

async fn recv(rx: &Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<AcpEvent>>>) -> AcpEvent {
    let mut g = rx.lock().await;
    tokio::time::timeout(Duration::from_secs(5), g.recv())
        .await
        .expect("event within timeout")
        .expect("channel open")
}

/// 6.1 建会话 + 完整回合：握手把会话 id、模型面与命令面板递出；一个 prompt
/// 产出 thinking/text/tool 增量与**唯一** Finished，且 pi 会话从不产生
/// PermissionRequest。
#[tokio::test]
async fn pi_session_completes_a_full_turn() {
    drain_journal("full-turn");
    let exe = write_script("fake-pi.sh", FAKE_PI);
    let mgr = pi_manager();
    let (sid, rx) = spawn_pi(&mgr, &exe, "", "pi-sess-A", "full-turn").await;

    // 握手后首事件 = 命令面板（会话建立时广播）。
    match recv(&rx).await {
        AcpEvent::AvailableCommands { commands, .. } => {
            assert_eq!(commands.len(), 1, "get_commands 应答 → 命令面板");
        }
        other => panic!("first event must be the command panel: {other:?}"),
    }

    mgr.send(
        &sid,
        AcpCommand::ContinueSession {
            session_id: sid.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("prompt accepted");

    let mut kinds: Vec<String> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "turn must finish; saw {kinds:?}");
        let evt = recv(&rx).await;
        match evt {
            AcpEvent::Finished { .. } => {
                kinds.push("finished".into());
                break;
            }
            AcpEvent::PermissionRequest { .. } => {
                panic!("pi 会话不得产生 PermissionRequest（无权限系统）")
            }
            AcpEvent::Error { terminal: true, .. } => panic!("unexpected terminal error"),
            other => kinds.push(kind_of(&other).to_string()),
        }
    }
    for expected in ["thinking", "text", "tool_start", "tool_end"] {
        assert!(kinds.contains(&expected.to_string()), "missing {expected}: {kinds:?}");
    }
    assert_eq!(
        kinds.iter().filter(|k| *k == "finished").count(),
        1,
        "agent_settled 是唯一 Finished：{kinds:?}"
    );

    // argv 钉住协议旗标。
    let argv = &journal_lines("full-turn")[0];
    assert!(argv.contains("--mode rpc"), "argv: {argv}");
    assert!(argv.contains("--session-dir"), "argv: {argv}");

    mgr.kill(&sid).await;
}

fn kind_of(e: &AcpEvent) -> &'static str {
    match e {
        AcpEvent::TextDelta { .. } => "text",
        AcpEvent::ThinkingDelta { .. } => "thinking",
        AcpEvent::ToolStart { .. } => "tool_start",
        AcpEvent::ToolProgress { .. } => "tool_progress",
        AcpEvent::ToolEnd { .. } => "tool_end",
        AcpEvent::UsageUpdate { .. } => "usage",
        _ => "other",
    }
}

/// 6.1 恢复：以 `--session <pi-id>` 重挂，`resumed=true`，agent 侧会话 id 回传。
#[tokio::test]
async fn pi_session_resumes_by_session_flag() {
    drain_journal("resume");
    let exe = write_script("fake-pi.sh", FAKE_PI);
    let mgr = pi_manager();
    let outcome = mgr
        .resume_session(
            "pi",
            vec![exe.to_string()],
            None,
            vec![
                ("FAKE_PI_SCENARIO".into(), "".into()),
                ("FAKE_PI_JOURNAL".into(), journal_path("resume")),
                ("FAKE_PI_SESSION".into(), "pi-sess-R".into()),
            ],
            "routing-old",
            Some("pi-sess-R".into()),
        )
        .await
        .expect("resume spawns");
    assert!(outcome.resumed, "resume must attach to the old conversation");
    assert_eq!(outcome.session_id, "routing-old", "routing id preserved on attach");
    assert_eq!(
        outcome.acp_session_id.as_deref(),
        Some("pi-sess-R"),
        "agent 侧会话 id 经握手上报"
    );
    let argv = &journal_lines("resume")[0];
    assert!(
        argv.contains("--session") && argv.contains("pi-sess-R"),
        "argv carries --session <id>: {argv}"
    );
    mgr.kill(&outcome.session_id).await;
}

/// 6.1 恢复被拒：桩见 `--session` 即退出 → 诚实回落新会话（新 routing id、
/// resumed=false），不伪装续接。
#[tokio::test]
async fn pi_rejected_resume_falls_back_to_fresh() {
    let reject = format!(
        "#!/bin/sh\nfor a in \"$@\"; do\n  [ \"$a\" = \"--session\" ] && {{ echo \"session not found\" >&2; exit 1; }}\ndone\n{}",
        FAKE_PI
    );
    let exe = write_script("fake-pi-reject.sh", &reject);
    drain_journal("rejected");
    let mgr = pi_manager();
    let outcome = mgr
        .resume_session(
            "pi",
            vec![exe.to_string()],
            None,
            vec![
                ("FAKE_PI_SCENARIO".into(), "".into()),
                ("FAKE_PI_JOURNAL".into(), journal_path("rejected")),
                ("FAKE_PI_SESSION".into(), "pi-fresh".into()),
            ],
            "routing-gone",
            Some("pi-sess-gone".into()),
        )
        .await
        .expect("fallback fresh must not error");
    assert!(!outcome.resumed, "回落后 resumed=false");
    assert_ne!(outcome.session_id, "routing-gone", "回落换新 routing id");
    assert_eq!(
        outcome.acp_session_id.as_deref(),
        Some("pi-fresh"),
        "回落会话上报 fresh 会话 id"
    );
    mgr.kill(&outcome.session_id).await;
}

/// 6.1 切模型：成功后 ModelChanged；失败非终态（会话存活）。
#[tokio::test]
async fn pi_model_switch_round_trips() {
    drain_journal("model");
    let exe = write_script("fake-pi.sh", FAKE_PI);
    let mgr = pi_manager();
    let (sid, rx) = spawn_pi(&mgr, &exe, "", "pi-sess-M", "model").await;
    // 丢弃命令面板首事件。
    let _ = recv(&rx).await;

    // 失败路径：已知 provider 的坏模型 id → 非终态 Error 带稳定标记。
    mgr.set_model(&sid, "anthropic/bad").await.expect("send ok");
    match recv(&rx).await {
        AcpEvent::Error { message, terminal, .. } => {
            assert!(!terminal);
            assert!(message.contains("bad"), "{message}");
            assert!(message.contains(sebas_acp::MODEL_UNCHANGED_MARKER), "{message}");
        }
        other => panic!("expected non-terminal error, got {other:?}"),
    }

    // 成功路径。
    mgr.set_model(&sid, "anthropic/m-2").await.expect("send ok");
    match recv(&rx).await {
        AcpEvent::ModelChanged { model_id, .. } => assert_eq!(model_id, "anthropic/m-2"),
        other => panic!("expected ModelChanged, got {other:?}"),
    }
    mgr.kill(&sid).await;
}

/// 6.1 取消：abort 下发 → `agent_settled(aborted=true)` 后 Finished。
#[tokio::test]
async fn pi_cancel_settles_before_finished() {
    drain_journal("cancel");
    let exe = write_script("fake-pi.sh", FAKE_PI);
    let mgr = pi_manager();
    let (sid, rx) = spawn_pi(&mgr, &exe, "", "pi-sess-C", "cancel").await;
    let _ = recv(&rx).await; // command panel

    mgr.send(&sid, AcpCommand::Cancel { session_id: sid.clone() })
        .await
        .expect("cancel sent");
    let mut saw_finished = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let AcpEvent::Finished { .. } = recv(&rx).await {
            saw_finished = true;
            break;
        }
    }
    assert!(saw_finished, "取消以 Finished 收尾");
    assert!(
        journal_lines("cancel").iter().any(|l| l.contains(r#""type":"abort""#)),
        "abort 命令确实下发"
    );
    mgr.kill(&sid).await;
}

/// 6.2 看门狗周期 `set_permission_mode` 探针在 pi 会话上被容忍：得到非终态
/// 「不支持」、会话存活（后续 prompt 照常开回合）。
#[tokio::test]
async fn pi_tolerates_watchdog_set_mode_probe() {
    drain_journal("watchdog");
    let exe = write_script("fake-pi.sh", FAKE_PI);
    let mgr = pi_manager();
    let (sid, rx) = spawn_pi(&mgr, &exe, "", "pi-sess-W", "watchdog").await;
    let _ = recv(&rx).await; // command panel

    mgr.send(
        &sid,
        AcpCommand::SetMode {
            session_id: sid.clone(),
            mode: "ask".into(),
        },
    )
    .await
    .expect("set mode delivered");
    match recv(&rx).await {
        AcpEvent::Error { message, terminal, .. } => {
            assert!(!terminal, "不支持必须是非终态");
            assert!(message.contains("无权限系统"), "{message}");
        }
        other => panic!("expected unsupported-mode error, got {other:?}"),
    }

    // 会话存活：后续 prompt 照常跑完整回合。
    mgr.send(
        &sid,
        AcpCommand::ContinueSession {
            session_id: sid.clone(),
            prompt: "still alive".into(),
        },
    )
    .await
    .expect("session still accepts prompts");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut finished = false;
    while Instant::now() < deadline {
        if let AcpEvent::Finished { .. } = recv(&rx).await {
            finished = true;
            break;
        }
    }
    assert!(finished, "SetMode 探针后会话仍可对话");
    mgr.kill(&sid).await;
}

/// 进程意外退出 → 带 terminal 标记的 Error（路由据此移除映射）。
#[tokio::test]
async fn pi_process_exit_is_terminal() {
    // 桩在第一个 prompt 后退出。
    let die = FAKE_PI.replace(
        r#"    *'"type":"prompt"'*)"#,
        r#"    *'"type":"prompt"'*) exit 0; ;; *'"type":"__never__"'*)"#,
    );
    let exe = write_script("fake-pi-die.sh", &die);
    drain_journal("die");
    let mgr = pi_manager();
    let (sid, rx) = spawn_pi(&mgr, &exe, "", "pi-sess-D", "die").await;
    let _ = recv(&rx).await; // command panel

    mgr.send(
        &sid,
        AcpCommand::ContinueSession {
            session_id: sid.clone(),
            prompt: "die now".into(),
        },
    )
    .await
    .expect("prompt sent");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut terminal = false;
    while Instant::now() < deadline {
        if let AcpEvent::Error { terminal: t, .. } = recv(&rx).await {
            terminal = t;
            break;
        }
    }
    assert!(terminal, "进程退出必须是 terminal Error");
    mgr.kill(&sid).await;
}
/// 真实 pi 二进制的握手形状校验（**skip-if-absent**）：pi 不在 PATH 上时
/// 跳过并如实说明（不伪装通过）。这是「桩照文档写、真实二进制才是权威」的
/// 对照——真实 pi 的 `get_state`/`get_available_models`/`get_commands` 应答
/// 必须被同一解析器吃下（多余字段忽略 = 前向兼容）。
#[tokio::test]
async fn real_pi_handshake_shape_matches_the_parser_when_present() {
    if which_pi().is_none() {
        eprintln!(
            "SKIP real_pi_handshake: `pi` 不在 PATH 上。装法：\
             npm i -g @earendil-works/pi-coding-agent（Node ≥22.19），\
             或把它放进测试 PATH 后重跑。"
        );
        return;
    }
    let mgr = pi_manager();
    // HOME 钉进临时目录（pi 自有状态落 ~/.pi）——绝不动操作员真实 pi 会话。
    let home = shared_dir().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let outcome = mgr
        .resume_session(
            "pi",
            vec!["pi".to_string()],
            None,
            vec![("HOME".into(), home.to_string_lossy().into_owned())],
            "real-probe",
            None,
        )
        .await
        .expect("real pi spawns in rpc mode");
    // 握手确实完成：agent 侧会话 id 非空（真实 pi 回 sessionId uuid）。
    let sid = outcome
        .acp_session_id
        .as_deref()
        .expect("real pi reports its session id");
    assert!(!sid.is_empty(), "real pi session id must be non-empty");
    mgr.kill(&outcome.session_id).await;
}

/// PATH 上找 `pi`（含 .exe/.cmd 后缀的 Windows 形态）。
fn which_pi() -> Option<std::path::PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &["", ".cmd", ".exe", ".bat"]
    } else {
        &[""]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in exts {
            let cand = dir.join(format!("pi{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}
