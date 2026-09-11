//! 真实 ACP 执行体的**进程级**验证（add-remote-execution-node 6.3 / 7.1 / 7.2）。
//!
//! 用的不是模型、也不是真 Claude Code，而是 `sebas-acp` 仓内的**测试替身**
//! `fake-claude-cli`（说话方式与 Claude Code 的 stream-json + control protocol
//! 一致）。它能做到真进程能做的关键几件事：被 spawn、走完 ACP 初始化握手、接受
//! 一轮 prompt、在 `perm` 提示下通过 `PreToolUse` 钩子**真的把权限请求交出来**、
//! 按决定执行或拒绝工具、然后结束这一轮。
//!
//! 因此这里验证的是：**节点侧执行体的真实子进程往返 + 门控全环**，不是模型质量。
//! 真 Claude Code 没有装在这台机器上，这一条在报告里如实说明。

use sebas_node::body::NodeBodyFactory;
use sebas_node::config::{AgentDriverKind, AgentSection, NodeBodyConfig, Upstream};
use sebas_node::session::SessionHost;
use sebas_node_link::{ApprovalDecision, SessionRejectCode};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `sebas-acp` 的测试替身二进制（由 `cargo build -p sebas-acp --bins` 产出）。
///
/// 找不到时不悄悄「通过」：打印明确的跳过说明并返回 `None`，由调用方决定怎么办
/// （这些测试选择跳过并把原因打出来，绝不假称验证过真进程）。
fn fake_claude() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    for name in ["fake-claude-cli", "fake-claude-cli.exe"] {
        let candidate = root.join("target/debug").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn body_config(fake: &PathBuf, enforces_mode: bool) -> NodeBodyConfig {
    let mut agents = BTreeMap::new();
    agents.insert(
        "claude".to_string(),
        AgentSection {
            command: Some(fake.to_string_lossy().to_string()),
            args: Vec::new(),
            driver: Some(AgentDriverKind::Claude),
            enforces_mode: Some(enforces_mode),
        },
    );
    NodeBodyConfig {
        agents,
        provider_profiles: BTreeMap::new(),
        default_provider: None,
        upstream: Upstream::Local,
        // 有项目目录的会话不需要它；这条路径另有用例覆盖。
        default_work_dir: None,
    }
}

/// 反复驱动宿主的传输层（把执行体攒下的条目收进日志），直到谓词成立或超时。
fn pump_until(
    host: &mut SessionHost,
    timeout: Duration,
    mut done: impl FnMut(&mut SessionHost) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        // 传一个「已经过了合并窗口」的时刻：强制把待上报批次结算掉，不留窗口等待。
        let _ = host.drain_events(Instant::now() + Duration::from_secs(1));
        if done(host) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn log_text(host: &mut SessionHost, session: &str, needle: &str) -> bool {
    host.log_from(session, 1)
        .map(|(_, entries, _)| entries.iter().any(|e| e.text.contains(needle)))
        .unwrap_or(false)
}

fn has_turn_finished(host: &mut SessionHost, session: &str) -> bool {
    host.log_from(session, 1)
        .map(|(_, entries, _)| entries.iter().any(|e| e.kind == "turn_finished"))
        .unwrap_or(false)
}

/// 一轮普通 turn：spawn 真子进程 → prompt → 输出进日志 → turn 结束。
#[test]
fn a_real_child_process_completes_a_turn_on_the_node() {
    let Some(fake) = fake_claude() else {
        eprintln!(
            "skipped: 找不到 target/debug/fake-claude-cli（先跑 `cargo build -p sebas-acp --bins`）\
             —— 本用例不声称验证了真实子进程"
        );
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let factory = Arc::new(NodeBodyFactory::new(body_config(&fake, true)));
    let mut host = SessionHost::new(sessions.path(), 4, 0, factory);

    let spawned = host
        .spawn(
            "s-1",
            Some(&project.path().to_string_lossy()),
            Some("claude"),
            None,
            Some("ask"),
            None,
        )
        .expect("真子进程应能建立会话");
    assert_eq!(spawned.agent_kind, "claude");
    assert_eq!(
        spawned.mode.as_deref(),
        Some("ask"),
        "claude 驱动有全量 PreToolUse 拦截点，配置声明能强制 → effective = desired"
    );

    host.prompt("s-1", "hello").expect("投递输入");

    assert!(
        pump_until(&mut host, Duration::from_secs(20), |h| has_turn_finished(
            h, "s-1"
        )),
        "真子进程应在超时内完成一轮 turn（日志：{:?}）",
        host.log_from("s-1", 1).unwrap().1
    );
    // 替身在 hello 场景下回 "hello " + "world"。
    assert!(
        log_text(&mut host, "s-1", "hello"),
        "子进程输出应进本地日志：{:?}",
        host.log_from("s-1", 1).unwrap().1
    );
}

/// 门控全环（真子进程）：权限请求上行 → 停驻（工具未执行）→ 决定下行 → agent 继续
/// → 工具结果落日志。这是 6.3「权限请求变成 GateRequest 且节点不自行决定」的
/// 端到端证据。
#[test]
fn a_permission_request_from_a_real_child_parks_then_resumes() {
    let Some(fake) = fake_claude() else {
        eprintln!(
            "skipped: 找不到 target/debug/fake-claude-cli（先跑 `cargo build -p sebas-acp --bins`）\
             —— 本用例不声称验证了真实子进程"
        );
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let factory = Arc::new(NodeBodyFactory::new(body_config(&fake, true)));
    let mut host = SessionHost::new(sessions.path(), 4, 0, factory);
    host.spawn(
        "s-1",
        Some(&project.path().to_string_lossy()),
        Some("claude"),
        None,
        Some("ask"),
        None,
    )
    .unwrap();

    host.prompt("s-1", "perm").unwrap();

    // 请求上行并停驻：`ask` 之下节点**不**自行决定。
    assert!(
        pump_until(&mut host, Duration::from_secs(20), |h| {
            !h.parked_approvals().is_empty()
        }),
        "真子进程的权限请求应停驻等待控制面（日志：{:?}）",
        host.log_from("s-1", 1).unwrap().1
    );
    let parked = host.parked_approvals();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].tool, "Bash", "工具名来自 agent 的权限请求");
    assert_eq!(
        parked[0].category,
        sebas_node_link::GateCategory::Execute,
        "Bash 应被归类为执行类"
    );
    // 未获准之前，工具结果不得出现。
    assert!(
        !log_text(&mut host, "s-1", "perm done"),
        "获准之前不得有工具执行痕迹"
    );

    // 决定下行 → agent 继续 → 工具结果进日志。
    let request_id = parked[0].request_id.clone();
    assert!(
        host.answer_approval("s-1", &request_id, ApprovalDecision::AllowOnce)
            .unwrap()
    );
    assert!(
        pump_until(&mut host, Duration::from_secs(20), |h| log_text(
            h, "s-1", "perm done"
        )),
        "放行后真子进程应继续并产出工具结果（日志：{:?}）",
        host.log_from("s-1", 1).unwrap().1
    );

    // 重复决议：可判别拒绝，不重复生效。
    assert_eq!(
        host.answer_approval("s-1", &request_id, ApprovalDecision::AllowOnce)
            .unwrap_err()
            .code,
        SessionRejectCode::UnknownApprovalRequest
    );
}

/// 强制不了 mode 的执行体如实回报 `None`（真子进程 + 未声明 enforces_mode）。
#[test]
fn a_real_child_without_declared_enforcement_reports_no_effective_mode() {
    let Some(fake) = fake_claude() else {
        eprintln!("skipped: 找不到 target/debug/fake-claude-cli");
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let factory = Arc::new(NodeBodyFactory::new(body_config(&fake, false)));
    let mut host = SessionHost::new(sessions.path(), 4, 0, factory);

    let spawned = host
        .spawn(
            "s-1",
            Some(&project.path().to_string_lossy()),
            Some("claude"),
            None,
            Some("ask"),
            None,
        )
        .unwrap();
    assert_eq!(
        spawned.mode, None,
        "节点配置没声明能强制 → 如实回「没有可声称生效的 mode」"
    );
    let (summary, ..) = host.snapshot("s-1").unwrap();
    assert_eq!(summary.desired_mode.as_deref(), Some("ask"));
    assert_eq!(summary.mode, None, "期望与实际不同，差异可见");
}

/// `set_model` / `cancel` 在真子进程上如实报告「做不到」：Claude 专用驱动没有
/// ACP 的 `set_config_option` 通道，节点不得把「命令发出去了」当成「已切换」。
#[test]
fn a_real_child_reports_unsupported_model_switch_and_idle_cancel() {
    let Some(fake) = fake_claude() else {
        eprintln!("skipped: 找不到 target/debug/fake-claude-cli");
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let factory = Arc::new(NodeBodyFactory::new(body_config(&fake, true)));
    let mut host = SessionHost::new(sessions.path(), 4, 0, factory);
    host.spawn(
        "s-1",
        Some(&project.path().to_string_lossy()),
        Some("claude"),
        None,
        Some("ask"),
        None,
    )
    .unwrap();

    // 没有在飞 turn：取消如实回 false，不假装取消了什么。
    assert!(
        !host.cancel("s-1").unwrap(),
        "空闲会话的取消应如实回 false"
    );

    // 模型切换：驱动明确说不支持 → 节点如实回错，且会话仍可继续用。
    let err = host.set_model("s-1", "some-model").unwrap_err();
    assert_eq!(err.code, SessionRejectCode::NodeError);
    assert!(
        err.cause.contains("不支持") || err.cause.contains("模型未变"),
        "成因要说明「模型没变」：{}",
        err.cause
    );
}

/// 无项目会话：工作目录来自节点配置的 `default_work_dir`（而不是节点进程的当前
/// 目录），并且没有配置时**如实拒绝**（见 body.rs 的单测）。
#[test]
fn a_project_less_session_runs_in_the_configured_default_work_dir() {
    let Some(fake) = fake_claude() else {
        eprintln!("skipped: 找不到 target/debug/fake-claude-cli");
        return;
    };
    let default_work = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut cfg = body_config(&fake, true);
    cfg.default_work_dir = Some(default_work.path().to_path_buf());
    let factory = Arc::new(NodeBodyFactory::new(cfg));
    let mut host = SessionHost::new(sessions.path(), 4, 0, factory);

    host.spawn("s-1", None, Some("claude"), None, Some("ask"), None)
        .expect("配了 default_work_dir 时无项目会话可落脚");
    host.prompt("s-1", "hello").unwrap();
    assert!(
        pump_until(&mut host, Duration::from_secs(20), |h| has_turn_finished(
            h, "s-1"
        )),
        "无项目会话应在默认工作目录里完成一轮"
    );
}

/// `upstream = control-plane-router` 且控制面没告知地址 → spawn 如实拒绝，
/// 成因指名缺的是什么（7.2；绝不猜地址）。
#[test]
fn the_router_upstream_without_an_advertised_address_is_refused() {
    let Some(fake) = fake_claude() else {
        eprintln!("skipped: 找不到 target/debug/fake-claude-cli");
        return;
    };
    let project = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut cfg = body_config(&fake, true);
    cfg.upstream = Upstream::ControlPlaneRouter;
    let factory = Arc::new(NodeBodyFactory::new(cfg));
    let mut host = SessionHost::new(sessions.path(), 4, 0, factory);

    let err = host
        .spawn(
            "s-1",
            Some(&project.path().to_string_lossy()),
            Some("claude"),
            None,
            Some("ask"),
            None,
        )
        .unwrap_err();
    assert_eq!(err.code, SessionRejectCode::ProviderUnavailable);
    assert!(err.cause.contains("router"), "{}", err.cause);
    assert!(
        err.cause.contains("没有") || err.cause.contains("未"),
        "成因要说明控制面没告知：{}",
        err.cause
    );
}
