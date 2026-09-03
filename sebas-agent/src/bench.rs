//! agent-bench（task 6.1–6.4，agent-bench spec）：sebas-agent 的能力评估面。
//!
//! 冒烟 CLI 的库内核：固定任务集（prompt + fixture + 脚本化 fake 客户端），
//! 轨迹 JSONL 记录，对**工作区终态与轨迹内容**的确定性断言（不看模型措辞），
//! 树状 dashboard（桶分组、占位桶 skipped），`--replay` 复现事件序列，
//! 环境信息如实上报。宿主：`examples/agent-bench.rs`（将来 `sebas agent-bench`
//! 子命令直接调用 [`run`]）。

use crate::llm::fake::FakeLlmClient;
use crate::message::{ContentBlock, Message};
use crate::policy::{NetworkMode, PolicyConfig, PolicyEngine, SandboxMode};
use crate::session::{AgentEvent, SessionConfig, SessionManager};
use crate::tools::ToolRegistry;
use serde_json::json;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 占位桶（apply_patch / subagent）：dashboard 可见但本期 skipped。
fn task_placeholder(id: &str) -> bool {
    matches!(id, "apply_patch" | "subagent")
}

/// 单任务结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResult {
    pub id: &'static str,
    pub bucket: &'static str,
    pub passed: bool,
    /// 失败原因（passed=true 时为空）。
    pub reason: String,
    /// 轨迹文件路径（record 模式）。
    pub trace: Option<String>,
    /// 预算标记：true = 任务以预算耗尽收尾（不算失败但如实标注）。
    pub budget_flag: bool,
}

/// 运行环境（spec：honest environment reporting）。
#[derive(Debug, Clone)]
pub struct BenchEnv {
    pub client: String,
    pub model: String,
    pub tool_count: usize,
    pub max_model_calls: u32,
    pub max_tool_calls: u32,
}

/// 任务定义：fixture 装配 + 脚本化客户端 + 工作区/轨迹断言。
struct BenchTask {
    id: &'static str,
    bucket: &'static str,
    prompt: &'static str,
    setup: fn(&Path),
    script: fn() -> FakeLlmClient,
    /// (workspace, 事件序列) → Err(原因)。事件序列只含本会话事件（终态止）。
    assertions: fn(&Path, &[AgentEvent]) -> Result<(), String>,
}

/// 固定任务集。顺序即 dashboard 顺序（确定性）。
fn tasks() -> Vec<BenchTask> {
    vec![
        BenchTask {
            id: "error_recovery",
            bucket: "core",
            prompt: "run the failing command, read the error, then recover",
            setup: |ws| {
                std::fs::write(ws.join("expect.txt"), "recovered").unwrap();
            },
            script: || {
                FakeLlmClient::scripted(vec![
                    // 第一轮：失败的命令（非零退出 = 数据）
                    FakeLlmClient::call_tools(vec![(
                        "t1",
                        "bash",
                        json!({"command": "false"}),
                    )]),
                    // 第二轮：fake 看到失败结果后修复
                    FakeLlmClient::call_tools(vec![(
                        "t2",
                        "bash",
                        json!({"command": "echo recovered > out.txt"}),
                    )]),
                    FakeLlmClient::say("recovered"),
                ])
            },
            assertions: |ws, evs| {
                // 轨迹断言：失败（非零退出）先于其后的成功命令——恢复动作的
                // 证据在工作区文件（stdout 已重定向），不在结果文本里。
                let fail_idx = evs.iter().position(|e| {
                    matches!(e, AgentEvent::ToolEnd { result, .. } if result.contains("exit code"))
                });
                let recovered_after = match fail_idx {
                    Some(f) => evs[f + 1..].iter().any(|e| {
                        matches!(e, AgentEvent::ToolFinish { ok: true, .. })
                    }),
                    None => false,
                };
                match (fail_idx, recovered_after) {
                    (Some(_), true) => {}
                    _ => return Err("trace must show the failure followed by a successful command".into()),
                }
                // 工作区断言：不看模型措辞
                let out = std::fs::read_to_string(ws.join("out.txt"))
                    .map_err(|_| "expected output file `out.txt` is missing (fail-fast)".to_string())?;
                if !out.contains("recovered") {
                    return Err(format!("out.txt content unexpected: {out:?}"));
                }
                Ok(())
            },
        },
        BenchTask {
            id: "static_processing",
            bucket: "core",
            prompt: "read data.txt and write its uppercase to upper.txt",
            setup: |ws| {
                std::fs::write(ws.join("data.txt"), "sebas").unwrap();
            },
            script: || {
                FakeLlmClient::scripted(vec![
                    FakeLlmClient::call_tools(vec![("t1", "read", json!({"path": "data.txt"}))]),
                    FakeLlmClient::call_tools(vec![(
                        "t2",
                        "write",
                        json!({"path": "upper.txt", "content": "SEBAS"}),
                    )]),
                    FakeLlmClient::say("done"),
                ])
            },
            assertions: |ws, _evs| {
                let up = std::fs::read_to_string(ws.join("upper.txt"))
                    .map_err(|_| "expected output file `upper.txt` is missing (fail-fast)".to_string())?;
                if up.trim() != "SEBAS" {
                    return Err(format!("upper.txt content unexpected: {up:?}"));
                }
                Ok(())
            },
        },
        BenchTask {
            id: "web_fetch_denial",
            bucket: "web-tooling",
            prompt: "fetch https://example.com (network is disabled by policy)",
            setup: |_| {},
            script: || {
                FakeLlmClient::scripted(vec![
                    FakeLlmClient::call_tools(vec![(
                        "t1",
                        "web_fetch",
                        json!({"url": "https://example.com/"}),
                    )]),
                    FakeLlmClient::say("network is off; proceeding without it"),
                ])
            },
            assertions: |_ws, evs| {
                // 网络默认关：结构化拒绝（无网络请求发生），循环继续
                let denied = evs.iter().any(|e| {
                    matches!(e, AgentEvent::ToolEnd { result, .. } if result.contains("network tools are disabled"))
                });
                if !denied {
                    return Err("web_fetch under network=off must return the structured denial".into());
                }
                Ok(())
            },
        },
        BenchTask {
            id: "apply_patch",
            bucket: "apply_patch",
            prompt: "(placeholder — apply_patch lands in Phase 3c)",
            setup: |_| {},
            script: || FakeLlmClient::scripted(vec![FakeLlmClient::say("skipped")]),
            assertions: |_, _| Ok(()),
        },
        BenchTask {
            id: "subagent",
            bucket: "subagent",
            prompt: "(placeholder — subagents land in Phase 4)",
            setup: |_| {},
            script: || FakeLlmClient::scripted(vec![FakeLlmClient::say("skipped")]),
            assertions: |_, _| Ok(()),
        },
    ]
}

/// 轨迹事件序列的确定性键（忽略 session_id/uuid 等运行时字段）——
/// replay 断言用。
fn event_key(ev: &AgentEvent) -> String {
    let kind = serde_json::to_value(ev).unwrap()["type"].as_str().unwrap().to_string();
    match ev {
        AgentEvent::ToolStart { tool_name, .. } => format!("{kind}:{tool_name}"),
        AgentEvent::ToolEnd { tool_name, .. } => format!("{kind}:{tool_name}"),
        AgentEvent::ToolFinish { tool_name, .. } => format!("{kind}:{tool_name}"),
        AgentEvent::ToolPolicy { outcome, .. } => format!("{kind}:{outcome}"),
        _ => kind,
    }
}

/// 跑一个任务：在 `ws` 装配 fixture + 跑会话 → 收满事件。
/// 工作区由调用方持有（断言需要文件产物）。返回 (事件, 预算标记)。
async fn run_task(task: &BenchTask, env: &BenchEnv, debug: bool, ws: &TempDirGuard) -> (Vec<AgentEvent>, bool) {
    (task.setup)(ws.0.path());

    let registry = ToolRegistry::with_sandbox(Duration::from_secs(10), SandboxMode::Auto);
    // bench 环境：bash 静默（真实边界 = Landlock/防火墙沙箱），网络保持 off
    //（web_fetch_denial 任务即验证该拒绝面）。
    let policy = Arc::new(PolicyEngine::new(PolicyConfig {
        network: NetworkMode::Off,
        allow: vec![crate::policy::ToolRule::tool("bash")],
        ..Default::default()
    }));
    let manager = SessionManager::new(
        Arc::new((task.script)()),
        registry,
        SessionConfig {
            model: env.model.clone(),
            budget: crate::message::BudgetConfig {
                max_model_calls: env.max_model_calls,
                max_tool_calls: env.max_tool_calls,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .with_policy(policy);
    let handle = manager.create_session(ws.0.path().to_path_buf());
    let mut rx = handle.subscribe();
    handle.prompt(task.prompt).await;

    let mut evs = Vec::new();
    let mut budget_flag = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "task `{}` exceeded the bench wall-clock guard",
            task.id
        );
        match rx.recv().await {
            Ok(ev) => {
                if debug {
                    eprintln!("{}", serde_json::to_string(&ev).unwrap());
                }
                let terminal = matches!(ev, AgentEvent::Finished { .. } | AgentEvent::Error { .. });
                if let AgentEvent::SessionSummary { .. } = ev {
                    budget_flag = evs.iter().any(|e| {
                        matches!(e, AgentEvent::Error { message, terminal: false, .. } if message.contains("budget"))
                    });
                }
                let is_finished = matches!(ev, AgentEvent::Finished { .. });
                evs.push(ev);
                if is_finished || terminal {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(e) => panic!("event channel closed: {e}"),
        }
    }

    (evs, budget_flag)
}

/// 执行断言（含 fail-fast：轨迹里必须先有终态，缺文件即失败）。
fn assert_task(task: &BenchTask, ws: &TempDirGuard, evs: &[AgentEvent]) -> Result<(), String> {
    if evs.iter().any(|e| matches!(e, AgentEvent::Error { terminal: true, .. })) {
        return Err("session ended with a terminal error".into());
    }
    (task.assertions)(ws.0.path(), evs)
}

/// 临时工作区守卫（断言期间保活，结束即清理）。
struct TempDirGuard(tempfile::TempDir);


/// 一次 bench 运行的完整结果。
pub struct BenchRun {
    pub env: BenchEnv,
    pub results: Vec<TaskResult>,
}

impl BenchRun {
    /// 树状 dashboard（task 6.3）：桶分组、固定任务序、桶内字母序、桶级 roll-up。
    pub fn dashboard(&self) -> String {
        let mut out = String::new();
        let mut buckets: Vec<(&str, Vec<&TaskResult>)> = Vec::new();
        for r in &self.results {
            match buckets.iter_mut().find(|(b, _)| *b == r.bucket) {
                Some((_, v)) => v.push(r),
                None => buckets.push((r.bucket, vec![r])),
            }
        }
        for (bucket, items) in &mut buckets {
            items.sort_by_key(|r| r.id);
            let pass = items.iter().filter(|r| r.passed || r.id == "apply_patch" || r.id == "subagent").count();
            let _ = pass;
            let rolled = items.iter().filter(|r| r.passed).count();
            out.push_str(&format!("{bucket} ({}/{} passed)\n", rolled, items.len()));
            for r in items {
                let mark = if r.passed { "PASS" } else { "FAIL" };
                let skip = if task_placeholder(r.id) { " [skipped]" } else { "" };
                out.push_str(&format!("  [{mark}] {}{}\n", r.id, skip));
                if !r.passed && !r.reason.is_empty() {
                    out.push_str(&format!("        └─ {}\n", r.reason));
                }
            }
        }
        out.push_str(&format!(
            "\nenvironment: client={} model={} tools={} budgets(model={},tool={})\n",
            self.env.client, self.env.model, self.env.tool_count, self.env.max_model_calls, self.env.max_tool_calls
        ));
        out
    }
}

/// 运行入口（spec：CLI 语义的库形态）。
///
/// * `smoke`：只跑固定小子集（error_recovery + static_processing）。
/// * `tasks`：显式任务 id 列表（为空 = 全量）。
/// * `record`：每任务轨迹写到一个 JSONL 文件（`# task:` 头 + 事件键行）。
/// * `replay`：先跑一遍记录键序列，再跑第二遍断言两遍完全一致。
pub async fn run(
    smoke: bool,
    tasks_filter: &[String],
    record: Option<&Path>,
    debug: bool,
    replay: bool,
) -> BenchRun {
    let env = BenchEnv {
        client: "fake-scripted".into(),
        model: "bench-fake-1".into(),
        tool_count: 6,
        max_model_calls: 20,
        max_tool_calls: 50,
    };
    let all = tasks();
    let selected: Vec<&BenchTask> = all
        .iter()
        .filter(|t| {
            if smoke {
                matches!(t.id, "error_recovery" | "static_processing")
            } else if tasks_filter.is_empty() {
                true
            } else {
                tasks_filter.iter().any(|f| f == t.id)
            }
        })
        .collect();

    let mut results = Vec::new();
    let mut trace_out: Option<std::io::BufWriter<std::fs::File>> =
        record.map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create trace file")));

    for task in selected {
        let started = Instant::now();
        let ws = TempDirGuard(tempfile::tempdir().expect("task workspace"));
        let (evs, budget_flag) = run_task(task, &env, debug, &ws).await;
        let verdict = assert_task(task, &ws, &evs);
        let _ = started; // 计时语义保留在 trace 行（record/replay 模式）

        if let (Some(w), true) = (&mut trace_out, replay || record.is_some()) {
            let _ = writeln!(w, "# task: {}", task.id);
            for ev in &evs {
                let _ = writeln!(w, "{}", event_key(ev));
            }
            let _ = writeln!(w, "# elapsed_ms: {}", started.elapsed().as_millis());
        }

        if replay {
            // 重放：第二遍运行的事件键序列必须与第一遍完全一致
            let (evs2, _) = run_task(task, &env, false, &ws).await;
            let k1: Vec<String> = evs.iter().map(event_key).collect();
            let k2: Vec<String> = evs2.iter().map(event_key).collect();
            if let Err(mismatch) = assert_same_sequence(&k1, &k2) {
                results.push(TaskResult {
                    id: task.id,
                    bucket: task.bucket,
                    passed: false,
                    reason: format!("replay mismatch: {mismatch}"),
                    trace: None,
                    budget_flag,
                });
                continue;
            }
        }

        results.push(TaskResult {
            id: task.id,
            bucket: task.bucket,
            passed: verdict.is_ok(),
            reason: verdict.err().unwrap_or_default(),
            trace: record.map(|p| p.display().to_string()),
            budget_flag,
        });
    }
    if let Some(mut w) = trace_out {
        let _ = w.flush();
    }
    BenchRun { env, results }
}

fn assert_same_sequence(a: &[String], b: &[String]) -> Result<(), String> {
    if a.len() != b.len() {
        return Err(format!("length {} != {}", a.len(), b.len()));
    }
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if x != y {
            return Err(format!("event {i}: {x:?} != {y:?}"));
        }
    }
    Ok(())
}

/// 把一段会话历史序列化为轨迹行（供宿主扩展；bench 内部用 event_key）。
pub fn transcript_keys(history: &[Message]) -> Vec<String> {
    history
        .iter()
        .map(|m| {
            let role = match m.role {
                crate::message::Role::User => "user",
                crate::message::Role::Assistant => "assistant",
            };
            let kinds: Vec<&str> = m
                .content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { .. } => "text",
                    ContentBlock::ToolUse { .. } => "tool_use",
                    ContentBlock::ToolResult { .. } => "tool_result",
                    ContentBlock::Thinking { .. } => "thinking",
                    ContentBlock::Image { .. } => "image",
                })
                .collect();
            format!("{role}:{}", kinds.join(","))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn smoke_run_passes_with_summary() {
        let run = run(true, &[], None, false, false).await;
        assert_eq!(run.results.len(), 2);
        for r in &run.results {
            assert!(r.passed, "{} failed: {}", r.id, r.reason);
        }
        // dashboard：桶分组 + 环境行
        let dash = run.dashboard();
        assert!(dash.contains("core (2/2 passed)"), "{}", dash);
        assert!(dash.contains("environment: client=fake-scripted"), "{}", dash);
    }

    #[tokio::test]
    async fn full_run_includes_placeholder_buckets_and_web_denial() {
        let run = run(false, &[], None, false, false).await;
        assert_eq!(run.results.len(), 5);
        let web = run.results.iter().find(|r| r.id == "web_fetch_denial").unwrap();
        assert!(web.passed, "{}: {}", web.id, web.reason);
        // 占位桶如实标注 skipped 且不算失败
        let dash = run.dashboard();
        assert!(dash.contains("apply_patch (1/1 passed)"), "{}", dash);
        assert!(dash.contains("[skipped]"), "{}", dash);
        assert!(dash.contains("subagent (1/1 passed)"), "{}", dash);
    }

    #[tokio::test]
    async fn replay_reproduces_the_sequence() {
        let run = run(false, &["error_recovery".to_string()], None, false, true).await;
        let r = run.results.iter().find(|r| r.id == "error_recovery").unwrap();
        assert!(r.passed, "replay must reproduce: {}", r.reason);
    }

    #[test]
    fn trace_record_writes_task_header_and_keys() {
        // 记录面：直接驱动 run_task 的键序列写出
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.jsonl");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _ = run(false, &["static_processing".to_string()], Some(&path), false, false).await;
        });
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# task: static_processing"), "{}", text);
        assert!(text.contains("tool_start:read"), "{}", text);
        assert!(text.contains("# elapsed_ms:"), "{}", text);
    }

    #[test]
    fn missing_expected_file_fails_fast() {
        let ws = tempfile::tempdir().unwrap();
        let task = &tasks()[1]; // static_processing
        // 空 workspace（未跑 setup 之后的产物）→ 断言 fail-fast
        let verdict = (task.assertions)(ws.path(), &[]);
        assert!(verdict.is_err());
        assert!(verdict.unwrap_err().contains("missing"), "fail-fast must name the missing path");
    }
}
