//! A programmable mock ACP agent (the agent/stdio side of ACP) used by the
//! ACP resume integration tests (openspec/changes/add-opencode-acp).
//!
//! It speaks the same `agent-client-protocol` wire as a real native-ACP agent
//! (e.g. opencode) but is scripted per scenario via argv `[scenario] [--journal PATH]`:
//! - `unavailable` → does NOT advertise `load_session`; every resume falls
//!   back to a fresh session.
//! - `load-fails`  → advertises `load_session`, but `session/load` responds
//!   with an error (conversation gone) — the driver must fall back to fresh.
//! - `load-ok`     → advertises `load_session` and `session/load` succeeds.
//!
//! The mock replies to prompts with `echo:<routing-id>` so tests can assert
//! that events are stamped with the *final* routing id (a fresh fallback
//! mints a new one, while a loaded session keeps the original) and that
//! prompts land on the right ACP session.
//!
//! `--journal PATH` lines: plain methods for setup-free scenarios plus the
//! `load_id` JSON for `session/load`, so tests can assert which id the driver
//! actually used to load (the routing id vs. the caller-provided ACP session
//! id — openspec/changes/add-acp-session-id-mapping).
//!
//! Model scenarios (add-acp-model-selection):
//! - `--model-options m1,m2,...` — return a `configOptions` `model` select on
//!   `session/new`/`session/load` with `currentValue = <first>`; the driver
//!   must surface `AcpModelInfo { current, options }` in the spawn outcome.
//! - `--reject-model m` — `session/set_config_option` for `m` is answered
//!   with an RPC error (invalid model); the driver must report a non-terminal
//!   `Error` and leave the session's current model unchanged.
//! - `session/set_config_option` is always journaled as
//!   `{"method":"session/set_config_option","session_id":...,"config_id":...,
//!   "value":...}` so tests assert the *wire* request (real ACP session id,
//!   `"model"` config id, the chosen value).

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse, PromptRequest,
    PromptResponse, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelectOption, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    StopReason, TextContent,
};
use agent_client_protocol::schema::v1::{SessionNotification, SessionUpdate};
use agent_client_protocol::{Agent, Error as AcpError, Stdio};
use std::sync::Arc;

const INITIALIZE: &str = "initialize";
const NEW_SESSION: &str = "session/new";
const LOAD_SESSION: &str = "session/load";
const PROMPT: &str = "session/prompt";
const SET_CONFIG_OPTION: &str = "session/set_config_option";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadBehavior {
    /// Advertise load_session as unavailable (resume must fall back).
    Unavailable,
    /// Advertise load_session but fail the load request (conversation gone).
    Fails,
    /// Advertise load_session and succeed.
    Succeeds,
}

fn parse_behavior(scenario: &str) -> LoadBehavior {
    match scenario {
        "unavailable" => LoadBehavior::Unavailable,
        "load-fails" => LoadBehavior::Fails,
        "load-ok" => LoadBehavior::Succeeds,
        other => {
            eprintln!("unknown scenario {other:?}; expected unavailable|load-fails|load-ok");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    let mut scenario = "load-ok".to_string();
    let mut journal: Option<std::path::PathBuf> = None;
    let mut hang_init = false;
    let mut model_options: Vec<String> = Vec::new();
    let mut reject_models: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--journal" => journal = args.next().map(std::path::PathBuf::from),
            "--hang-init" => hang_init = true,
            "--model-options" => {
                model_options = args
                    .next()
                    .map(|s| s.split(',').map(str::to_string).collect())
                    .unwrap_or_default();
            }
            "--reject-model" => {
                if let Some(m) = args.next() {
                    reject_models.push(m);
                }
            }
            _ => scenario = a,
        }
    }
    run(parse_behavior(&scenario), journal, hang_init, model_options, reject_models).await;
}

/// Build the model `configOptions` entry returned with new/load responses
/// when `--model-options` was given. `currentValue` = 第一个选项（与
/// `SessionConfigOption::select` 的语义一致）。
fn model_config_options(model_options: &[String]) -> Option<Vec<SessionConfigOption>> {
    if model_options.is_empty() {
        return None;
    }
    let current = model_options[0].clone();
    let items: Vec<SessionConfigSelectOption> = model_options
        .iter()
        .map(|m| SessionConfigSelectOption::new(m.clone(), m.clone()))
        .collect();
    Some(vec![SessionConfigOption::select(
        "model",
        "Model",
        current,
        items,
    )
    .category(SessionConfigOptionCategory::Model)])
}

async fn run(
    behavior: LoadBehavior,
    journal: Option<std::path::PathBuf>,
    hang_init: bool,
    model_options: Vec<String>,
    reject_models: Vec<String>,
) {
    let journal = Arc::new(journal);
    let log = |method: &str| {
        if let Some(p) = journal.as_ref() {
            let line = format!(r#"{{"method":"{method}"}}"#);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p.as_path())
            {
                use std::io::Write;
                let _ = writeln!(f, "{line}");
            }
        }
    };
    let log_load = |method: &str, load_id: &str| {
        if let Some(p) = journal.as_ref() {
            let line = format!(
                r#"{{"method":"{method}","load_id":{}}}"#,
                serde_json::to_string(load_id).unwrap_or_else(|_| "\"\"".to_string())
            );
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p.as_path())
            {
                use std::io::Write;
                let _ = writeln!(f, "{line}");
            }
        }
    };
    // 记录 `session/set_config_option` 的 wire 请求细节：真实 ACP 会话 id、
    // config id、值——测试断言 SetModel 命令确实发了标准请求。
    let log_set_config = |session_id: &str, config_id: &str, value: &str| {
        if let Some(p) = journal.as_ref() {
            let line = format!(
                r#"{{"method":"{SET_CONFIG_OPTION}","session_id":{},"config_id":{},"value":{}}}"#,
                serde_json::to_string(session_id).unwrap_or_default(),
                serde_json::to_string(config_id).unwrap_or_default(),
                serde_json::to_string(value).unwrap_or_default(),
            );
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p.as_path())
            {
                use std::io::Write;
                let _ = writeln!(f, "{line}");
            }
        }
    };

    Agent
        .builder()
        .name("sebas-mock-acp")
        .on_receive_request(
            {
                async move |req: InitializeRequest, responder, _cx| {
                    if hang_init {
                        // Never answer initialize: exercises the manager's
                        // startup-timeout handshake wait.
                        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                        return Ok(());
                    }
                    log(INITIALIZE);
                    let caps = match behavior {
                        LoadBehavior::Unavailable => AgentCapabilities::new(),
                        LoadBehavior::Fails | LoadBehavior::Succeeds => {
                            AgentCapabilities::new().load_session(true)
                        }
                    };
                    responder.respond(
                        InitializeResponse::new(req.protocol_version).agent_capabilities(caps),
                    )
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let model_config = model_config_options(&model_options);
                async move |_req: NewSessionRequest, responder, _cx| {
                    log(NEW_SESSION);
                    // The ACP session id is application-chosen; mint a fresh
                    // one so tests can tell "new" sessions apart from the
                    // loaded conversation id.
                    let sid = format!("acp-new-{}", nanoid());
                    let mut resp = NewSessionResponse::new(sid);
                    if let Some(opts) = &model_config {
                        resp = resp.config_options(opts.clone());
                    }
                    responder.respond(resp)
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let model_config = model_config_options(&model_options);
                async move |req: LoadSessionRequest, responder, _cx| {
                    // Record the id the driver actually asked to load — a
                    // test asserts the driver uses the caller-provided real
                    // ACP session id, not the routing id.
                    log_load(LOAD_SESSION, &req.session_id.to_string());
                    match behavior {
                        LoadBehavior::Fails => responder
                            .respond_with_error(AcpError::internal_error().data("no such session")),
                        LoadBehavior::Succeeds => {
                            let mut resp = LoadSessionResponse::new();
                            if let Some(opts) = &model_config {
                                resp = resp.config_options(opts.clone());
                            }
                            responder.respond(resp)
                        }
                        LoadBehavior::Unavailable => {
                            // Should never be reached; fail loudly if it is.
                            responder.respond_with_error(
                                AcpError::method_not_found().data("load_session not advertised"),
                            )
                        }
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let reject_models = reject_models.clone();
                let model_config = model_config_options(&model_options);
                async move |req: SetSessionConfigOptionRequest, responder, _cx| {
                    // Journal the wire request: session_id 必须走真实 ACP 会话
                    // id（非路由 id）、configId 必须是 "model"、value 是所选项。
                    let value = match &req.value {
                        agent_client_protocol::schema::v1::SessionConfigOptionValue::ValueId { value } => {
                            value.0.to_string()
                        }
                        other => format!("{other:?}"),
                    };
                    log_set_config(
                        &req.session_id.to_string(),
                        &req.config_id.0,
                        &value,
                    );
                    // 脚本化拒绝无效 model：返回 RPC 错误（等价于 agent 拒绝）。
                    if reject_models.iter().any(|m| m == &value) {
                        responder
                            .respond_with_error(
                                AcpError::invalid_params().data("invalid model id"),
                            )?;
                        return Ok(());
                    }
                    // 成功：回显最新 configOptions（currentValue 更新为所选项）。
                    let mut opts = model_config.clone().unwrap_or_default();
                    if let Some(first) = opts.first_mut()
                        && let agent_client_protocol::schema::v1::SessionConfigKind::Select(
                            sel,
                        ) = &mut first.kind
                    {
                        sel.current_value =
                            agent_client_protocol::schema::v1::SessionConfigValueId::new(
                                value.clone(),
                            );
                    }
                    responder.respond(SetSessionConfigOptionResponse::new(opts))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                async move |req: PromptRequest, responder, cx| {
                    log(PROMPT);
                    // Reply with the routing id so tests can assert which
                    // session the prompt actually landed on.
                    let sid = req.session_id.to_string();
                    let _ = cx.send_notification(SessionNotification::new(
                        sid.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(format!("echo:{sid}")),
                        ))),
                    ));
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
        .expect("mock acp agent exits cleanly");
}

/// Tiny id generator (avoid pulling a uuid dep into the mock).
fn nanoid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}