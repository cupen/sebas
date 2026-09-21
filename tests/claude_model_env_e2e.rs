//! End-to-end coverage of the Claude model-setting feature.
//!
//! Two layers, one journey (workbench-composer-input-polish 2.x +
//! acp-claude-model-env-cover):
//!
//! 1. **env 注入层**：`sebas::spawn_env::resolve_spawn_overrides` 算出来的
//!    `extra_env`（endpoint 键 `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`
//!    + 5 键 cover env `ANTHROPIC_MODEL` / `ANTHROPIC_DEFAULT_OPUS_MODEL` /
//!    `ANTHROPIC_DEFAULT_SONNET_MODEL` / `ANTHROPIC_DEFAULT_HAIKU_MODEL` /
//!    `CLAUDE_CODE_SUBAGENT_MODEL`）真的进了 fake-claude 子进程，由
//!    fake-claude 启动期 journal `meta.env` 行回读断言。
//! 2. **SetModel 控制协议层**：环境里钉了默认模型（cover env 把 opus/sonnet
//!    都覆盖成 deepseek-v4）后，`mgr.set_model(...)` 走 cc-agent-sdk 的
//!    set_model 控制协议依然把 assistant 帧 model 字段从默认值切到请求值；
//!    切回 "default" 后 fake 的 init 帧观察值 "fake" 覆盖乐观值（D5 自愈）。
//!
//! 整个测试与既有 `sebas-acp/tests/claude_model_selection.rs` 的形态
//! 对齐：fake-claude binary 通过 `--journal PATH` 把 argv/env/frame 写盘，
//! 断言用轮询读 journal。

use sebas::spawn_env::resolve_spawn_overrides;
use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use sebas_acp::claude::ClaudeCodeDriver;
use sebas_dispatch::provider_state::{ProviderMode, ProviderRuntimeState};
use sebas_dispatch::state_store::DefaultSelection;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// 串行化所有与 `state_store` / `SEBAS_ROUTER_PROVIDER_OVERLAY` 相关的
/// 全局副作用（与 `src/spawn_env.rs` 测试同惯例）。env var 跨进程可见，
/// 并发跑会让别的测试误读错的值。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn workspace_target() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug")
}

fn fake() -> PathBuf {
    workspace_target().join(format!("fake-claude-cli{}", std::env::consts::EXE_SUFFIX))
}

/// 每个测试独享的 journal 路径（并行测试互不串写）。
fn journal(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sebas-model-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("journal dir");
    dir.join(format!("{tag}.jsonl"))
}

/// 轮询 journal 直到出现 `meta_env` 行（fake-claude 启动期写入），返回
/// `msg.env` 子对象。
fn wait_meta_env(journal_path: &PathBuf) -> serde_json::Value {
    for _ in 0..200 {
        if let Ok(content) = std::fs::read_to_string(journal_path) {
            for line in content.lines() {
                if !line.contains("\"dir\":\"meta_env\"")
                    || !line.contains("ANTHROPIC_BASE_URL")
                {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(env) = v.pointer("/msg/env").cloned() {
                        return env;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "no meta_env line in {} within 10s",
        journal_path.display()
    );
}

/// 写一个临时 overlay 文件覆盖 `state_store::load()` 的查找路径，再 export
/// provider 用的 API key 环境变量。RAII guard 在析构时还原全局 env 状态。
struct EnvGuard {
    _dir: tempfile::TempDir,
    prev_overlay: Option<String>,
    api_key_env: String,
}

impl EnvGuard {
    fn new(provider: &str, api_key_env: &str, api_key_value: &str, overlay_body: &str) -> Self {
        // `provider` is documentation for the call site — asserts that
        // overlay_body actually names it (catches copy-paste typos).
        assert!(
            overlay_body.contains(&format!("\"{provider}\"")),
            "overlay body must mention provider {provider:?}"
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let overlay_path = dir.path().join("providers.json");
        std::fs::write(&overlay_path, overlay_body).expect("write overlay");
        let prev_overlay = std::env::var("SEBAS_ROUTER_PROVIDER_OVERLAY").ok();
        // SAFETY: ENV_LOCK held across the whole test body.
        unsafe {
            std::env::set_var(
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
                overlay_path.to_str().unwrap(),
            );
            std::env::set_var(api_key_env, api_key_value);
        }
        Self {
            _dir: dir,
            prev_overlay,
            api_key_env: api_key_env.to_string(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: ENV_LOCK held across the whole test body.
        unsafe {
            match &self.prev_overlay {
                Some(v) => std::env::set_var("SEBAS_ROUTER_PROVIDER_OVERLAY", v),
                None => std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY"),
            }
            std::env::remove_var(&self.api_key_env);
        }
    }
}

async fn drain_until<F: Fn(&AcpEvent) -> bool>(
    mgr: &SessionManager,
    id: &str,
    pred: F,
) -> AcpEvent {
    for _ in 0..20 {
        let evt = tokio::time::timeout(Duration::from_secs(5), mgr.next_event(id))
            .await
            .expect("event timeout")
            .expect("stream open");
        if pred(&evt) {
            return evt;
        }
    }
    panic!("no matching event within 20 reads");
}

/// 第一关：Direct 模式 + deepseek provider，resolve_spawn_overrides 输出
/// 的 (extra_env, extra_args) 完整到达 claude 子进程。
///
/// fake-claude 的 `meta.env` journal 行把这 7 个键原样记下：
///   - ANTHROPIC_BASE_URL, ANTHROPIC_AUTH_TOKEN（来自 driver.resolve_env 的
///     Direct+Anthropic 分支——preset "deepseek" 给出 base_url_anthropic）
///   - ANTHROPIC_MODEL, ANTHROPIC_DEFAULT_OPUS_MODEL, ANTHROPIC_DEFAULT_SONNET_MODEL,
///     ANTHROPIC_DEFAULT_HAIKU_MODEL, CLAUDE_CODE_SUBAGENT_MODEL（5 键 cover env）
///
/// 不存在「env 算对了但 child 没拿到」的失败模式（SDK 把 extra_env 拼进
/// Command::envs，理论上必到）——但理论上的事情就是要拿真进程验证一次。
#[tokio::test]
async fn direct_provider_injects_endpoint_and_5_key_cover_env_into_claude_child() {
    let _g = ENV_LOCK.lock().unwrap();
    let journal_path = journal("cover_env");
    let _ = std::fs::remove_file(&journal_path);

    let state = ProviderRuntimeState {
        mode: ProviderMode::Direct {
            provider: "deepseek".into(),
        },
        default_selection: Some(DefaultSelection::new("deepseek")),
    };
    // preset = "deepseek" 让 compute_provider_resolution 从内置预设拿
    // base_url_anthropic；models 自定义覆盖（preset 默认 models 也行，
    // 这里用自定义短列表 + 显式能力标注保证映射稳定）。
    // Overlay 形状必须包 `providers` 顶层键（state_store::OverlayWire
    // 只 deserialize 这个 key；裸 `{deepseek: …}` 会被静默丢空）。
    let _env_guard = EnvGuard::new(
        "deepseek",
        "DEEPSEEK_API_KEY",
        "sk-test-deepseek",
        r#"{
            "providers": {
                "deepseek": {
                    "preset": "deepseek",
                    "api_key_env": "DEEPSEEK_API_KEY",
                    "models": [
                        {"id": "deepseek-v4-pro"},
                        {"id": "deepseek-v4-flash"}
                    ]
                }
            }
        }"#,
    );

    // 走真实调用链：resolve_spawn_overrides 是 session_boot::spawn_overrides
    // 的核心，被 acp_spawn_and_activate / acp_resume_and_activate 用。
    let (extra_env, extra_args) =
        resolve_spawn_overrides(&ClaudeCodeDriver, &state, None);

    // extra_env 至少 6 个键：2 endpoint + 5 cover = 7。ModelEntry 解析
    // 失败会让 cover 段为空（compute_provider_resolution 的 fallback），
    // 故先 assert 再 spawn。
    let keys: std::collections::HashSet<&str> =
        extra_env.iter().map(|(k, _)| k.as_str()).collect();
    for required in [
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "CLAUDE_CODE_SUBAGENT_MODEL",
    ] {
        assert!(
            keys.contains(required),
            "resolve_spawn_overrides must produce {required}, got keys={keys:?}"
        );
    }
    let auth = extra_env
        .iter()
        .find(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN")
        .map(|(_, v)| v.as_str())
        .expect("checked above");
    assert_eq!(
        auth, "sk-test-deepseek",
        "API key must propagate from env var to ANTHROPIC_AUTH_TOKEN"
    );

    // Spawn fake-claude with the resolved env + args.
    let mut command = vec![fake().to_str().unwrap().to_string()];
    command.push("--journal".into());
    command.push(journal_path.to_str().unwrap().into());
    command.extend(extra_args.iter().cloned());

    let mgr = SessionManager::claude_only(Duration::from_secs(15));
    let _session_id = mgr
        .create_claude_session(
            command[0].as_str(),
            command[1..].to_vec(),
            None,
            extra_env.clone(),
            "".into(),
        )
        .await
        .expect("spawn");

    // fake-claude dumps `meta.env` at startup (before any prompt arrives),
    // so we can read it immediately without driving a turn.
    let env_dump = wait_meta_env(&journal_path);
    let s = |k: &str| env_dump[k].as_str().unwrap_or("<missing>").to_string();

    // Endpoint keys
    assert!(
        s("ANTHROPIC_BASE_URL").starts_with("http"),
        "BASE_URL must be a URL string, got {:?}",
        env_dump["ANTHROPIC_BASE_URL"]
    );
    assert_eq!(
        s("ANTHROPIC_AUTH_TOKEN"),
        "sk-test-deepseek",
        "AUTH_TOKEN must match the api_key_env value"
    );

    // 5-key cover env: strong→weak ordering with two models
    assert_eq!(
        s("ANTHROPIC_MODEL"),
        "deepseek-v4-pro",
        "strongest model → ANTHROPIC_MODEL"
    );
    assert_eq!(
        s("ANTHROPIC_DEFAULT_OPUS_MODEL"),
        "deepseek-v4-pro",
        "strongest → OPUS"
    );
    assert_eq!(
        s("ANTHROPIC_DEFAULT_SONNET_MODEL"),
        "deepseek-v4-flash",
        "weakest → SONNET when only 2 models"
    );
    assert_eq!(
        s("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        "deepseek-v4-flash",
        "weakest → HAIKU"
    );
    assert_eq!(
        s("CLAUDE_CODE_SUBAGENT_MODEL"),
        "deepseek-v4-flash",
        "subagent reuses weakest tier"
    );

    // ANTHROPIC_API_KEY is not injected by sebas; null is correct.
    assert!(
        env_dump["ANTHROPIC_API_KEY"].is_null(),
        "ANTHROPIC_API_KEY must remain unset (sebas only injects AUTH_TOKEN); got {:?}",
        env_dump["ANTHROPIC_API_KEY"]
    );

    let _ = std::fs::remove_file(&journal_path);
}

/// 第二关：cover env 注入之后，set_model 控制协议仍把帧 model 切到请求值。
///
/// 验证「env 钉默认模型」与「运行时切模型」不冲突：cover env 给的是
/// "deepseek-v4-pro"，但 fake-claude 跑 hello 场景时 set_model("opus")
/// 切完下一回合报 "opus"；切回 "default" 时 fake 的 init 帧 "fake" 覆盖
/// 乐观值（D5 自愈）。
#[tokio::test]
async fn set_model_after_cover_env_takes_effect_from_next_turn() {
    let _g = ENV_LOCK.lock().unwrap();
    let journal_path = journal("set_model_under_cover");
    let _ = std::fs::remove_file(&journal_path);

    let _env_guard = EnvGuard::new(
        "deepseek",
        "DEEPSEEK_API_KEY",
        "sk-test-deepseek",
        r#"{
            "providers": {
                "deepseek": {
                    "preset": "deepseek",
                    "api_key_env": "DEEPSEEK_API_KEY",
                    "models": [
                        {"id": "deepseek-v4-pro"},
                        {"id": "deepseek-v4-flash"}
                    ]
                }
            }
        }"#,
    );

    let state = ProviderRuntimeState {
        mode: ProviderMode::Direct {
            provider: "deepseek".into(),
        },
        default_selection: Some(DefaultSelection::new("deepseek")),
    };
    let (extra_env, extra_args) =
        resolve_spawn_overrides(&ClaudeCodeDriver, &state, None);

    let mut command = vec![fake().to_str().unwrap().to_string()];
    command.push("--journal".into());
    command.push(journal_path.to_str().unwrap().into());
    command.extend(extra_args);

    let mgr = SessionManager::claude_only(Duration::from_secs(15));
    let session_id = mgr
        .create_claude_session(
            command[0].as_str(),
            command[1..].to_vec(),
            None,
            extra_env,
            "".into(),
        )
        .await
        .expect("spawn");

    // 1) First turn: init 帧 model = "fake" → ModelChanged("fake").
    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("first prompt");
    let evt = drain_until(&mgr, &session_id, |e| {
        matches!(e, AcpEvent::ModelChanged { .. })
    })
    .await;
    assert!(
        matches!(evt, AcpEvent::ModelChanged { ref model_id, .. } if model_id == "fake"),
        "first turn's init frame must observe the agent's actual model 'fake', got {evt:?}"
    );
    drain_until(&mgr, &session_id, |e| matches!(e, AcpEvent::Finished { .. })).await;

    // 2) Switch to "opus": optimistic ModelChanged("opus"), fake's assistant
    //    frame next turn reports "opus".
    mgr.set_model(&session_id, "opus").await.expect("set opus");
    let evt = drain_until(&mgr, &session_id, |e| {
        matches!(e, AcpEvent::ModelChanged { .. })
    })
    .await;
    assert!(
        matches!(evt, AcpEvent::ModelChanged { ref model_id, .. } if model_id == "opus"),
        "optimistic update must announce the requested model, got {evt:?}"
    );

    // 3) Switch back to "default": optimistic ModelChanged("default"); next
    //    turn's init-frame observation "fake" must overwrite the optimistic
    //    value (D5 self-heal). The cover env in the child process is
    //    irrelevant here — fake's init frame still reports "fake" because
    //    it ignores the cover env (it's not a real claude).
    mgr.set_model(&session_id, "default").await.expect("set default");
    let evt = drain_until(&mgr, &session_id, |e| {
        matches!(e, AcpEvent::ModelChanged { .. })
    })
    .await;
    assert!(
        matches!(evt, AcpEvent::ModelChanged { ref model_id, .. } if model_id == "default"),
        "switching to default first emits optimistic default, got {evt:?}"
    );

    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: "hello".into(),
        },
    )
    .await
    .expect("third prompt");
    let evt = drain_until(&mgr, &session_id, |e| {
        matches!(e, AcpEvent::ModelChanged { .. })
    })
    .await;
    assert!(
        matches!(evt, AcpEvent::ModelChanged { ref model_id, .. } if model_id == "fake"),
        "frame observation must overwrite optimistic 'default' with the agent's real model; \
         cover env does not change fake's reported model, got {evt:?}"
    );
    drain_until(&mgr, &session_id, |e| matches!(e, AcpEvent::Finished { .. })).await;

    let _ = std::fs::remove_file(&journal_path);
}