//! [`PiDriver`]：驱动 `pi --mode rpc` 子进程（stdio 严格 JSONL）的第三个
//! 一等 agent driver（add-pi-driver）。
//!
//! 职责拆分：
//! - 协议编解码/翻译在 [`super::codec`]（纯函数，fixture 单测锁形状）；
//! - 本模块做进程编排：argv 组装（`--mode rpc`、`--session-dir` 钉住、恢复
//!   期 `--session <id>` 重挂）、握手（`get_state` 学习 pi 会话 id +
//!   `get_available_models` 模型面 + `get_commands` 命令面板，startup
//!   timeout 内完不成即杀进程失败）、双泵（stdout 持续读——停读会拖死 pi
//!   的背压；stderr 只记日志不解析）、事件/命令回路。
//!
//! 语义要点（design D2–D5 / spec `pi-agent`）：
//! - 回合边界唯一锚在 `agent_settled`：`agent_end` 后的自动重试、compaction
//!   恢复、排队消息都在同一回合内，`Finished` 只在 settled 后发出。
//! - 取消 = RPC `abort` + 等 `agent_settled(aborted=true)` 后发 `Finished`。
//! - 恢复 = spawn `--session <id>`；被拒（进程退出 / `get_state` 失败）诚实
//!   回落新会话（新 routing id + resumed=false），不伪造续接。
//! - v1 无权限系统：不产生 `PermissionRequest`；`SetMode` 非终态「不支持」。
//! - 进程意外退出 → 带 terminal 标记的 `Error`。

use super::codec::{
    FrameDecoder, PiEvent, PiFrame, PiRequest, PiResponse, Translator, encode_request,
    next_request_id, parse_commands_data, parse_frame, parse_models_data, parse_state_data,
    split_model_id, translate_command, translate_event, translate_set_model_failure,
};
use crate::agent_driver::{AgentDriver, DriverConfig, DriverError, DriverHandle};
use crate::session::{AcpCommand, AcpEvent, AcpModelInfo, AvailableCommand};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc;

/// pi agent 的 sessions 目录缺省（pi 自身文档的默认落点）。
pub const DEFAULT_PI_SESSIONS_DIR: &str = "~/.pi/agent/sessions";

/// The [`crate::AgentDriver`] implementation driving `pi --mode rpc`.
///
/// `sessions_dir` 是该驱动实例的 per-agent 配置（装配点从
/// `[acp.agents.<name>] sessions_dir` 注入，镜像 `ClaudeDriver::with_models`
/// 的实例配置先例）：spawn argv 以 `--session-dir` 钉住它，pi 会话文件天然
/// 持久，恢复经 `--session <id>` 重挂。
#[derive(Debug, Clone, Default)]
pub struct PiDriver {
    sessions_dir: Option<String>,
}

impl PiDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// 该 agent 实例的 sessions 目录（`None` = 不加 `--session-dir`，pi 用
    /// 自身缺省——config 装配点恒传 `Some`）。
    pub fn with_sessions_dir(sessions_dir: Option<String>) -> Self {
        Self { sessions_dir }
    }
}

/// 一次 spawn 尝试的参数：被拒回落后 `resume` 翻 false、routing id 换新。
struct Attempt {
    resume: bool,
    session_id: String,
}

/// try_spawn 的失败分类：`ResumeRejected` 触发诚实回落（fresh 重试一次）。
enum HandshakeError {
    ResumeRejected(String),
    Timeout(std::time::Duration),
    Other(anyhow::Error),
}

/// try_spawn 成功产物：握手学到的会话身份 + 运行期资源。
struct Spawned {
    child: Child,
    stdin: ChildStdin,
    frame_rx: mpsc::Receiver<String>,
    buffered: Vec<PiFrame>,
    pi_session_id: String,
    model: Option<AcpModelInfo>,
    commands: Vec<AvailableCommand>,
}

#[async_trait::async_trait]
impl AgentDriver for PiDriver {
    async fn spawn(&self, cfg: DriverConfig) -> Result<DriverHandle, DriverError> {
        let mut attempt = Attempt {
            resume: cfg.resume,
            session_id: cfg.session_id.clone(),
        };
        loop {
            match try_spawn(self, &cfg, &attempt).await {
                Ok(spawned) => {
                    return Ok(
                        build_handle(cfg, spawned, attempt.session_id, attempt.resume).await
                    );
                }
                // 恢复被拒（会话文件缺失/损坏 → pi 进程退出或握手失败）：
                // 诚实回落新会话——新 routing id + resumed=false + 告警，不
                // 向上游伪装续接成功（spec「rejected resume falls back」）。
                // 只有「本来就在恢复」时才回落新会话；fresh 启动的握手期
                // 进程退出不是恢复被拒，如实失败——否则会无限重试。
                Err(HandshakeError::ResumeRejected(reason)) if attempt.resume => {
                    let fresh = uuid::Uuid::new_v4().to_string();
                    tracing::warn!(
                        kind = %cfg.kind_slug,
                        old = %attempt.session_id,
                        fresh = %fresh,
                        reason = %reason,
                        "pi rejected resume; falling back to a fresh session",
                    );
                    attempt = Attempt {
                        resume: false,
                        session_id: fresh,
                    };
                }
                Err(HandshakeError::ResumeRejected(reason)) => {
                    return Err(DriverError::Other(anyhow::anyhow!(
                        "pi handshake failed on a fresh spawn: {reason}"
                    )));
                }
                Err(HandshakeError::Timeout(d)) => return Err(DriverError::Timeout(d)),
                Err(HandshakeError::Other(e)) => return Err(DriverError::Other(e)),
            }
        }
    }
}

/// 子进程 argv：`<exe> [operator args…] --mode rpc [--session-dir <dir>]
/// [--session <id>]`。协议旗标由驱动组装（operator args 只经 config `args`
/// 透传，键值形式）。
fn assemble_argv(
    command: &[String],
    sessions_dir: Option<&str>,
    resume_id: Option<&str>,
) -> Vec<String> {
    let mut argv = command.to_vec();
    argv.push("--mode".into());
    argv.push("rpc".into());
    if let Some(dir) = sessions_dir.filter(|s| !s.is_empty()) {
        argv.push("--session-dir".into());
        argv.push(dir.into());
    }
    if let Some(id) = resume_id {
        argv.push("--session".into());
        argv.push(id.into());
    }
    argv
}

/// spawn + 握手（`get_state` / `get_available_models` / `get_commands` 三连，
/// `pi.id` 关联、startup timeout 兜底）。握手期间到达的会话事件被缓冲，
/// 随 run 循环重放（订阅先于 prompt，不丢快速完成的回合尾巴）。
async fn try_spawn(
    driver: &PiDriver,
    cfg: &DriverConfig,
    attempt: &Attempt,
) -> Result<Spawned, HandshakeError> {
    let resume_id = attempt
        .resume
        .then(|| cfg.load_session_id.clone().unwrap_or_else(|| attempt.session_id.clone()));
    let argv = assemble_argv(&cfg.command, driver.sessions_dir.as_deref(), resume_id.as_deref());
    let Some((exe, args)) = argv.split_first() else {
        return Err(HandshakeError::Other(anyhow::anyhow!("empty pi command")));
    };
    // Windows：npm 形态的无扩展名 CLI 需解析成 .exe/.cmd/.bat 才能 spawn。
    let exe = crate::resolve_windows_executable(exe);

    let mut child = spawn_with_etxtbsy_retry(&exe, args, cfg).await?;

    let mut stdin = child.stdin.take().expect("piped stdin");
    // stderr 只作诊断（D8）：逐行记日志，绝不解析。
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            use tokio::io::AsyncBufReadExt;
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(stderr = %line, "pi child");
            }
        });
    }
    // stdout 持续读：字节泵 → LF 分帧 → 帧通道（停读会拖死 pi 的背压）。
    let stdout = child.stdout.take().expect("piped stdout");
    let (frame_tx, mut frame_rx) = mpsc::channel::<String>(64);
    tokio::spawn(read_stdout(stdout, frame_tx));

    // 握手三连并发发出，按 id 收应答（pi 命令处理是异步的，允许乱序）。
    let mut hs_seq = 0u64;
    let state_id = next_request_id(&mut hs_seq);
    let models_id = next_request_id(&mut hs_seq);
    let commands_id = next_request_id(&mut hs_seq);
    for req in [
        PiRequest::GetState { id: state_id.clone() },
        PiRequest::GetAvailableModels {
            id: models_id.clone(),
        },
        PiRequest::GetCommands {
            id: commands_id.clone(),
        },
    ] {
        write_request(&mut stdin, &req)
            .await
            .map_err(|e| HandshakeError::Other(anyhow::anyhow!("pi stdin 写入失败: {e}")))?;
    }

    let handshake = collect_handshake(&mut frame_rx, &state_id, &models_id, &commands_id);
    let (buffered, pi_session_id, model, commands) =
        match tokio::time::timeout(cfg.startup_timeout, handshake).await {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                let _ = child.start_kill();
                return Err(e);
            }
            Err(_) => {
                let _ = child.start_kill();
                return Err(HandshakeError::Timeout(cfg.startup_timeout));
            }
        };
    tracing::debug!(
        kind = %cfg.kind_slug,
        pi_session_id = %pi_session_id,
        models = model.as_ref().map(|m| m.options.len()).unwrap_or(0),
        "pi handshake completed",
    );
    Ok(Spawned {
        child,
        stdin,
        frame_rx,
        buffered,
        pi_session_id,
        model,
        commands,
    })
}

/// 收拢三个握手应答；进程退出/应答失败按恢复语义分类（恢复尝试被拒 →
/// [`HandshakeError::ResumeRejected`]，由上层回落新会话）。
async fn collect_handshake(
    frame_rx: &mut mpsc::Receiver<String>,
    state_id: &str,
    models_id: &str,
    commands_id: &str,
) -> Result<(Vec<PiFrame>, String, Option<AcpModelInfo>, Vec<AvailableCommand>), HandshakeError> {
    let mut state: Option<PiResponse> = None;
    let mut models: Option<PiResponse> = None;
    let mut commands: Option<PiResponse> = None;
    let mut buffered: Vec<PiFrame> = Vec::new();
    let classify = |msg: String| -> HandshakeError {
        HandshakeError::Other(anyhow::anyhow!("pi handshake failed: {msg}"))
    };
    loop {
        let Some(line) = frame_rx.recv().await else {
            // stdout EOF = 进程退出。恢复尝试下最常见的原因就是会话文件
            // 缺失/损坏（pi 启动即退）；诚实归类为「被拒」让上层回落。
            return Err(HandshakeError::ResumeRejected(
                "pi process exited before the handshake completed".into(),
            ));
        };
        let Some(frame) = parse_frame(&line) else {
            continue;
        };
        let PiFrame::Response(resp) = &frame else {
            buffered.push(frame);
            continue;
        };
        match resp.id.as_deref() {
            Some(id) if id == state_id => state = Some(resp.clone()),
            Some(id) if id == models_id => models = Some(resp.clone()),
            Some(id) if id == commands_id => commands = Some(resp.clone()),
            _ => {}
        }
        if state.is_some() && models.is_some() && commands.is_some() {
            break;
        }
    }

    let state = state.expect("loop broke with all three responses");
    if !state.success {
        // `get_state` 失败：恢复尝试（--session 指到坏文件）与其它启动期
        // 错误都可能——按被拒归类，恢复路径回落新会话、fresh 路径由上层
        // 兜底重试一次后如实失败。
        let reason = state.error.unwrap_or_else(|| "get_state failed".into());
        return Err(HandshakeError::ResumeRejected(reason));
    }
    let Some(data) = &state.data else {
        return Err(classify("get_state response carries no data".into()));
    };
    let Some(parsed) = parse_state_data(data) else {
        return Err(classify("get_state data lacks sessionId".into()));
    };
    let models_list = models
        .filter(|r| r.success)
        .and_then(|r| r.data)
        .map(|d| parse_models_data(&d))
        .unwrap_or_default();
    let model = build_model_info(parsed.current_model.as_ref(), &models_list);
    let commands = commands
        .filter(|r| r.success)
        .and_then(|r| r.data)
        .map(|d| parse_commands_data(&d))
        .unwrap_or_default();
    Ok((buffered, parsed.session_id, model, commands))
}

fn work_dir_or_cwd(work_dir: &Option<String>) -> std::path::PathBuf {
    work_dir
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
        })
}

/// spawn pi 子进程。Linux 上若二进制文件仍有写句柄（刚写完脚本即 exec、
/// 或包管理器正在原地替换二进制），`execve` 返回 `ETXTBSY`——这是瞬态竞态，
/// 短暂退避后重试；其余错误照旧分类（NotFound 给可读文案）。
async fn spawn_with_etxtbsy_retry(
    exe: &str,
    args: &[String],
    cfg: &DriverConfig,
) -> Result<Child, HandshakeError> {
    const ATTEMPTS: u32 = 20;
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..ATTEMPTS {
        match build_command(exe, args, cfg).spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.raw_os_error() == Some(26) => {
                // ETXTBSY
                last = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(5 * (attempt as u64 + 1))).await;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(HandshakeError::Other(anyhow::anyhow!(
                    "pi binary not found or not runnable: {exe}"
                )));
            }
            Err(e) => return Err(HandshakeError::Other(e.into())),
        }
    }
    Err(HandshakeError::Other(anyhow::anyhow!(
        "pi binary busy after {ATTEMPTS} attempts: {}",
        last.map(|e| e.to_string()).unwrap_or_default()
    )))
}

fn build_command(exe: &str, args: &[String], cfg: &DriverConfig) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .current_dir(work_dir_or_cwd(&cfg.work_dir))
        .envs(cfg.extra_env.iter().cloned())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    cmd
}

async fn write_request(stdin: &mut ChildStdin, req: &PiRequest) -> std::io::Result<()> {
    stdin.write_all(encode_request(req).as_bytes()).await?;
    stdin.flush().await
}

/// stdout 字节泵：持续读（背压友好）→ LF 分帧 → 通道。EOF/错误即结束
/// （run 循环把通道关闭视作进程退出终态）。
async fn read_stdout(mut stdout: impl tokio::io::AsyncRead + Unpin, frame_tx: mpsc::Sender<String>) {
    let mut decoder = FrameDecoder::new();
    let mut buf = [0u8; 8192];
    loop {
        match stdout.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                decoder.push(&buf[..n]);
                // 必须抽干本次 read 里的**全部**完整帧——`while let ... && …`
                // 会在首个 send 成功时因条件为假直接退出循环，把同批到达的
                // 后续帧（如 message_update 之后的 agent_settled）永久留在
                // 解码器缓冲里。
                while let Some(frame) = decoder.next_frame() {
                    if frame_tx.send(frame).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

/// `provider/id` wire 词汇的模型选择面：options 来自 `get_available_models`
/// 应答（不内置硬编码模型表），current 来自 `get_state.model`（可缺省——
/// 空模型面 = `None`，webui 不显示下拉）。
fn build_model_info(
    current: Option<&(String, String)>,
    models: &[(String, String)],
) -> Option<AcpModelInfo> {
    let mut options: Vec<String> = models
        .iter()
        .map(|(p, id)| format!("{p}/{id}"))
        .collect();
    let current = current.map(|(p, id)| format!("{p}/{id}"));
    if options.is_empty() && current.is_none() {
        return None;
    }
    if let Some(cur) = &current
        && !options.contains(cur)
    {
        options.insert(0, cur.clone());
    }
    Some(AcpModelInfo {
        current: current.unwrap_or_default(),
        options,
    })
}

/// 把握手产物组装成 [`DriverHandle`] 并启动事件/命令回路。
async fn build_handle(
    cfg: DriverConfig,
    spawned: Spawned,
    routing_id: String,
    resumed: bool,
) -> DriverHandle {
    let Spawned {
        child,
        stdin,
        frame_rx,
        buffered,
        pi_session_id,
        model,
        commands,
    } = spawned;
    let session_id = routing_id;

    // 命令面板在会话建立时广播一次（与 claude 驱动同模式）；空表 = 无面板
    // 的诚实退化。
    let _ = cfg
        .evt_tx
        .send(AcpEvent::AvailableCommands {
            session_id: session_id.clone(),
            commands,
        })
        .await;

    let run = run_loop(
        child,
        stdin,
        frame_rx,
        buffered,
        cfg.evt_tx,
        session_id.clone(),
        cfg.cmd_rx,
        cfg.cancel_rx,
        cfg.terminal_sent,
    );

    DriverHandle {
        session_id,
        resumed,
        // pi 会话 id 经握手上报（对齐通用 ACP 驱动的 acp_session_id 语义），
        // 恢复以它为 `--session <id>` 目标。
        acp_session_id: Some(pi_session_id),
        model,
        handshake: None,
        run: Box::pin(run),
    }
}

/// 事件发送：terminal 事件置 `terminal_sent`（manager 包装任务不再合成第
/// 二条「agent process exited」）。返回 false = 通道关闭，回路退出。
async fn send_evt(
    evt_tx: &mpsc::Sender<AcpEvent>,
    terminal_sent: &Arc<AtomicBool>,
    evt: AcpEvent,
) -> bool {
    let is_terminal = matches!(evt, AcpEvent::Error { terminal: true, .. });
    if is_terminal {
        terminal_sent.store(true, Ordering::SeqCst);
    }
    evt_tx.send(evt).await.is_ok() && !is_terminal
}

/// 事件/命令回路。退出条件：kill（cancel）、命令通道关闭、带 terminal 标记
/// 的 `Error`、pi 进程退出（stdout EOF）。
#[allow(clippy::too_many_arguments)]
async fn run_loop(
    mut child: Child,
    mut stdin: ChildStdin,
    mut frame_rx: mpsc::Receiver<String>,
    buffered: Vec<PiFrame>,
    evt_tx: mpsc::Sender<AcpEvent>,
    session_id: String,
    mut cmd_rx: mpsc::Receiver<AcpCommand>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
    terminal_sent: Arc<AtomicBool>,
) {
    let mut translator = Translator::new();
    // 回合在飞（prompt 已受理，等 agent_settled）；disposition=handled 的
    // prompt 不开回合、应答即收尾。
    let mut turn_active = false;
    // 取消在途：abort 已发，等 settled(aborted=true) 后发 Finished。
    let mut canceling = false;
    // 在途 prompt / set_model 的请求 id（应答携带 disposition / 成败）。
    let mut prompt_id: Option<String> = None;
    let mut set_model_pending: Option<(String, String)> = None;
    // 裸模型 id → provider 的已知表（set_model 需要 provider；`provider/id`
    // 词汇直接拆分，兜底查表）。
    let mut model_index: HashMap<String, String> = HashMap::new();
    let mut req_seq = 0u64;

    let stdin_dead = |session_id: &str| {
        AcpEvent::Error {
            session_id: session_id.to_string(),
            message: "pi 进程写入失败".into(),
            terminal: true,
        }
    };

    // 重放握手期间缓冲的会话事件。
    for frame in buffered {
        let PiFrame::Event(ev) = frame else { continue };
        if !handle_event(
            &ev,
            &session_id,
            &mut translator,
            &mut turn_active,
            &mut canceling,
            &evt_tx,
            &terminal_sent,
        )
        .await
        {
            return;
        }
    }

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => break, // kill：退出循环即收尾子进程
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    AcpCommand::CreateSession { prompt, .. }
                    | AcpCommand::ContinueSession { prompt, .. } => {
                        // 排队语义（steer/follow_up）v1 不消费（D2）：sebas 每
                        // 会话同时只跑一回合，prompt 直发（映射单源在 codec
                        // 的 translate_command）。
                        let Some(req) = translate_command(
                            &AcpCommand::ContinueSession {
                                session_id: session_id.clone(),
                                prompt,
                            },
                            &mut req_seq,
                        ) else {
                            continue;
                        };
                        turn_active = true;
                        canceling = false;
                        translator.begin_turn();
                        prompt_id = Some(req.id().to_string());
                        if write_request(&mut stdin, &req).await.is_err() {
                            send_evt(&evt_tx, &terminal_sent, stdin_dead(&session_id)).await;
                            break;
                        }
                    }
                    AcpCommand::Cancel { .. } => {
                        let req = PiRequest::Abort { id: next_request_id(&mut req_seq) };
                        canceling = true;
                        if write_request(&mut stdin, &req).await.is_err() {
                            send_evt(&evt_tx, &terminal_sent, stdin_dead(&session_id)).await;
                            break;
                        }
                    }
                    AcpCommand::SetModel { model_id, .. } => {
                        // `provider/id` 直接拆；裸 id 查握手期模型表；仍无则
                        // 如实报错（不猜 provider）。
                        let req = match split_model_id(&model_id) {
                            Some((provider, id)) => Some(PiRequest::SetModel {
                                id: next_request_id(&mut req_seq),
                                provider,
                                model_id: id,
                            }),
                            None => model_index.get(&model_id).map(|provider| {
                                PiRequest::SetModel {
                                    id: next_request_id(&mut req_seq),
                                    provider: provider.clone(),
                                    model_id: model_id.clone(),
                                }
                            }),
                        };
                        match req {
                            Some(r) => {
                                set_model_pending = Some((r.id().to_string(), model_id));
                                if write_request(&mut stdin, &r).await.is_err() {
                                    send_evt(&evt_tx, &terminal_sent, stdin_dead(&session_id)).await;
                                    break;
                                }
                            }
                            None => {
                                let _ = send_evt(&evt_tx, &terminal_sent, AcpEvent::Error {
                                    session_id: session_id.clone(),
                                    message: format!(
                                        "set model 需 provider/id 形式（或握手期已知的模型 id），{}",
                                        crate::MODEL_UNCHANGED_MARKER
                                    ),
                                    terminal: false,
                                }).await;
                            }
                        }
                    }
                    AcpCommand::SetMode { mode, .. } => {
                        // pi 无权限系统（D4）：非终态「不支持」，会话存活——
                        // 看门狗周期 set_permission_mode 探针因此被容忍。
                        let _ = send_evt(&evt_tx, &terminal_sent, AcpEvent::Error {
                            session_id: session_id.clone(),
                            message: format!(
                                "set mode {mode:?} 需要支持权限模式的执行体；pi agent 无权限系统，不支持，模式未变"
                            ),
                            terminal: false,
                        }).await;
                    }
                    AcpCommand::PermissionReply { .. } => {
                        // pi 会话不产生 PermissionRequest；应答走 pending map，
                        // 这里不该收到。
                        tracing::debug!("ignoring unexpected PermissionReply on pi session");
                    }
                }
            }
            line = frame_rx.recv() => {
                let Some(line) = line else {
                    // stdout EOF = 进程退出：terminal Error，路由移除映射。
                    send_evt(&evt_tx, &terminal_sent, AcpEvent::Error {
                        session_id: session_id.clone(),
                        message: "pi 进程意外退出".into(),
                        terminal: true,
                    }).await;
                    break;
                };
                let Some(frame) = parse_frame(&line) else { continue };
                match frame {
                    PiFrame::Response(resp) => {
                        if prompt_id.as_deref() == resp.id.as_deref() {
                            prompt_id = None;
                            let disposition = resp
                                .data
                                .as_ref()
                                .and_then(|d| d.get("disposition"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("");
                            if !resp.success {
                                // prompt 被拒（未受理）：非终态 Error + Finished
                                // 收尾回合边界（回合确实没开起来）。
                                let reason = resp.error.as_deref().unwrap_or("prompt rejected");
                                if !send_evt(&evt_tx, &terminal_sent, AcpEvent::Error {
                                    session_id: session_id.clone(),
                                    message: format!("pi prompt 被拒: {reason}"),
                                    terminal: false,
                                }).await { break; }
                                turn_active = false;
                                if !send_evt(&evt_tx, &terminal_sent, AcpEvent::Finished {
                                    session_id: session_id.clone(),
                                }).await { break; }
                            } else if disposition == "handled" {
                                // 扩展命令/输入处理器消费了 prompt，没有 run
                                // 会开始——不会有 agent_settled，应答即收尾。
                                turn_active = false;
                                if !send_evt(&evt_tx, &terminal_sent, AcpEvent::Finished {
                                    session_id: session_id.clone(),
                                }).await { break; }
                            }
                            // started/queued：等 agent_settled（回合边界）。
                        } else if set_model_pending.as_ref().map(|(id, _)| id.as_str())
                            == resp.id.as_deref()
                        {
                            let (_, model_id) = set_model_pending.take().expect("checked above");
                            if resp.success {
                                // 新模型进索引（后续裸 id set_model 可查）。
                                if let Some(data) = &resp.data
                                    && let (Some(p), Some(m)) = (
                                        data.get("provider").and_then(serde_json::Value::as_str),
                                        data.get("id").and_then(serde_json::Value::as_str),
                                    )
                                {
                                    model_index.insert(m.to_string(), p.to_string());
                                }
                                if !send_evt(&evt_tx, &terminal_sent, AcpEvent::ModelChanged {
                                    session_id: session_id.clone(),
                                    model_id,
                                }).await { break; }
                            } else {
                                let reason = resp.error.as_deref().unwrap_or("set_model failed");
                                if !send_evt(&evt_tx, &terminal_sent,
                                    translate_set_model_failure(&session_id, &model_id, reason),
                                ).await { break; }
                            }
                        } else {
                            tracing::debug!(
                                command = ?resp.command,
                                "pi response without a waiting request",
                            );
                        }
                    }
                    PiFrame::Event(ev) => {
                        if !handle_event(
                            &ev,
                            &session_id,
                            &mut translator,
                            &mut turn_active,
                            &mut canceling,
                            &evt_tx,
                            &terminal_sent,
                        ).await {
                            break;
                        }
                    }
                }
            }
        }
    }
    // 收尾：请求有序停机（关 stdin），随后强杀兜底（kill_on_drop 亦在）。
    let _ = stdin.shutdown().await;
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// 处理一条会话事件。返回 false = 回路必须退出（terminal 事件 / 通道关闭）。
async fn handle_event(
    ev: &PiEvent,
    session_id: &str,
    translator: &mut Translator,
    turn_active: &mut bool,
    canceling: &mut bool,
    evt_tx: &mpsc::Sender<AcpEvent>,
    terminal_sent: &Arc<AtomicBool>,
) -> bool {
    match ev {
        PiEvent::AgentSettled { .. } => {
            // 唯一回合边界：重试/compaction 恢复/排队消息都结束之后才到这
            // 里（spec「retries finish before the turn settles」）；取消路径
            // 同样以 settled(aborted=true) 收尾再发 Finished。
            if *turn_active || *canceling {
                *turn_active = false;
                *canceling = false;
                return send_evt(
                    evt_tx,
                    terminal_sent,
                    AcpEvent::Finished {
                        session_id: session_id.to_string(),
                    },
                )
                .await;
            }
            true
        }
        PiEvent::AgentEnd { will_retry } => {
            // 低层 run 结束：willRetry 时后续还有重试 run，同一回合继续。
            if will_retry.unwrap_or(false) {
                tracing::info!(session_id, "pi agent run ended, retry scheduled");
            }
            true
        }
        PiEvent::Other(kind) => {
            // compaction / auto_retry / summarization_retry / queue_update 等：
            // 不发明新 AcpEvent（D2），如实记日志、归入回合内呈现。
            if kind.starts_with("compaction_")
                || kind.starts_with("auto_retry_")
                || kind.starts_with("summarization_retry_")
            {
                tracing::info!(session_id, kind = %kind, "pi session event (turn continues)");
            } else {
                tracing::debug!(session_id, kind = %kind, "pi session event (unmapped)");
            }
            true
        }
        _ => {
            for evt in translate_event(session_id, ev, translator) {
                if !send_evt(evt_tx, terminal_sent, evt).await {
                    return false;
                }
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_driver::DriverConfig;
    use crate::session::AcpEvent;
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot};

    /// fake pi 二进制（POSIX sh 脚本）：讲握手 + 一个剧本回合。场景与日志
    /// 路径经 env（FAKE_PI_SCENARIO / FAKE_PI_JOURNAL / FAKE_PI_SESSION）
    /// 注入——DriverConfig.extra_env 正好承载，不污染 argv 断言。
    const FAKE_PI: &str = r#"#!/bin/sh
journal() { if [ -n "$FAKE_PI_JOURNAL" ]; then printf '%s\n' "$1" >>"$FAKE_PI_JOURNAL"; fi; }
rid() { printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'; }

journal "argv: $*"

case "$FAKE_PI_SCENARIO" in
  hang) sleep 60; exit 0 ;;
esac

while IFS= read -r line; do
  journal "$line"
  id=$(rid "$line")
  case "$line" in
    *'"type":"get_state"'*)
      printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"%s","model":{"id":"m-1","provider":"anthropic"}}}\n' "$id" "$FAKE_PI_SESSION"
      ;;
    *'"type":"get_available_models"'*)
      printf '{"id":"%s","type":"response","command":"get_available_models","success":true,"data":{"models":[{"id":"m-1","provider":"anthropic"},{"id":"g-5","provider":"openai"}]}}\n' "$id"
      ;;
    *'"type":"get_commands"'*)
      printf '{"id":"%s","type":"response","command":"get_commands","success":true,"data":{"commands":[{"name":"fix-tests","description":"Fix failing tests"}]}}\n' "$id"
      [ "$FAKE_PI_SCENARIO" = die_after_handshake ] && exit 0
      ;;
    *'"type":"prompt"'*)
      if [ "$FAKE_PI_SCENARIO" = die_after_handshake ]; then exit 0; fi
      printf '{"id":"%s","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}\n' "$id"
      printf '{"type":"message_update","usage":{"input":10,"output":1,"cacheRead":0,"cacheWrite":0},"assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"hmm"}}\n'
      printf '{"type":"message_update","usage":{"input":10,"output":2,"cacheRead":0,"cacheWrite":0},"assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"hello world"}}\n'
      printf '{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"ls"}}\n'
      printf '{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[{"type":"text","text":"done"}]},"isError":false}\n'
      printf '{"type":"agent_end","messages":[],"willRetry":false}\n'
      printf '{"type":"agent_settled","aborted":false}\n'
      ;;
    *'"type":"abort"'*)
      printf '{"type":"agent_settled","aborted":true}\n'
      printf '{"id":"%s","type":"response","command":"abort","success":true}\n' "$id"
      ;;
    *'"type":"set_model"'*)
      case "$line" in
        *'"modelId":"bad"'*)
          printf '{"id":"%s","type":"response","command":"set_model","success":false,"error":"Model not found: bad"}\n' "$id"
          ;;
        *)
          printf '{"id":"%s","type":"response","command":"set_model","success":true,"data":{"id":"m-2","provider":"anthropic"}}\n' "$id"
          ;;
      esac
      ;;
  esac
done
"#;

    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// 进程级共享的 fake 脚本目录。
    fn shared_script_dir() -> &'static std::path::Path {
        static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let d = std::env::temp_dir().join(format!("sebas-pi-tests-{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            d
        })
    }

    /// 取共享脚本路径：**每个脚本全进程只写一次**。测试并发跑时，若各测试
    /// 各自写自己的脚本副本，会与另一个测试的 `execve` 撞上 Linux 的写-执行
    /// 竞态（ETXTBSY，overlayfs/tmpfs 上尤其明显）。日志路径仍每测试独立
    /// （经 env 注入），脚本内容与路径无关。
    fn shared_script(
        name: &str,
        body: &str,
        slot: &'static std::sync::OnceLock<String>,
    ) -> String {
        slot.get_or_init(|| write_script(shared_script_dir(), name, body))
            .clone()
    }

    static FAKE_PI_EXE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static FAKE_PI_REJECT_EXE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

    struct Fixture {
        dir: tempfile::TempDir,
        journal: String,
        exe: String,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("journal.log").display().to_string();
        Fixture {
            exe: shared_script("fake-pi.sh", FAKE_PI, &FAKE_PI_EXE),
            journal,
            dir,
        }
    }

    struct Harness {
        handle: DriverHandle,
        cmd_tx: mpsc::Sender<AcpCommand>,
        evt_rx: mpsc::Receiver<AcpEvent>,
        cancel_tx: tokio::sync::oneshot::Sender<()>,
        /// run loop 的 JoinHandle。`DriverHandle::run` 是**未启动**的 future
        /// （生产里由 `SessionManager` `tokio::spawn`），测试须自行启动，
        /// 否则命令进不去、事件出不来。
        run_task: tokio::task::JoinHandle<()>,
    }

    /// 以指定剧本拉起 PiDriver（fake pi 经 env 拿场景/日志/会话 id）。
    #[allow(clippy::too_many_arguments)]
    async fn harness(
        exe: &str,
        journal: &str,
        scenario: &str,
        pi_session: &str,
        resume: bool,
        load_session_id: Option<String>,
        sessions_dir: Option<String>,
        startup_timeout: Duration,
    ) -> Result<Harness, DriverError> {
        let driver = PiDriver::with_sessions_dir(sessions_dir);
        let (evt_tx, evt_rx) = mpsc::channel(256);
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let cfg = DriverConfig {
            kind_slug: "pi".into(),
            command: vec![exe.to_string()],
            work_dir: None,
            extra_env: vec![
                ("FAKE_PI_SCENARIO".into(), scenario.into()),
                ("FAKE_PI_JOURNAL".into(), journal.into()),
                ("FAKE_PI_SESSION".into(), pi_session.into()),
            ],
            session_id: "routing-1".into(),
            load_session_id,
            resume,
            startup_timeout,
            evt_tx,
            cmd_rx,
            cancel_rx,
            pending_perms: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            terminal_sent: Arc::new(AtomicBool::new(false)),
        };
        let mut handle = driver.spawn(cfg).await?;
        // 生产契约：`SessionManager` 负责 spawn run loop（见 manager.rs）。
        // 测试自行启动，模拟 manager 的这一步。
        let run = std::mem::replace(&mut handle.run, Box::pin(async {}));
        let run_task = tokio::spawn(handle_run(run));
        Ok(Harness {
            handle,
            cmd_tx,
            evt_rx,
            cancel_tx,
            run_task,
        })
    }

    /// 把 `DriverHandle::run` 的 boxed future 转成可 spawn 的 async 块。
    async fn handle_run(run: futures::future::BoxFuture<'static, ()>) {
        run.await;
    }

    /// 等 run loop 收尾（命令通道关闭 / cancel / terminal）。带超时兜底：
    /// 挂死时 panic 并指明，而不是让测试无限等待。
    async fn join_run(task: tokio::task::JoinHandle<()>) {
        match tokio::time::timeout(Duration::from_secs(10), task).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => panic!("run loop panicked: {e}"),
            Err(_) => panic!("run loop did not finish within 10s"),
        }
    }

    fn journal_lines(journal: &str) -> Vec<String> {
        std::fs::read_to_string(journal)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// 消费会话建立时广播的命令面板事件（`build_handle` 在 spawn 返回前就
    /// 发了它，是所有用例的首事件）。
    async fn drain_command_panel(evt_rx: &mut mpsc::Receiver<AcpEvent>) {
        match evt_rx.recv().await {
            Some(AcpEvent::AvailableCommands { .. }) => {}
            other => panic!("expected the command panel first, got {other:?}"),
        }
    }

    /// 2.1 握手四元组上报：routing id 不变、resumed=false、pi 会话 id 与
    /// 模型面（provider/id 词汇，来自 get_available_models 应答）随 handle
    /// 递出；argv 含 `--mode rpc` 与 `--session-dir`。
    #[tokio::test]
    async fn handshake_reports_session_id_and_model_surface() {
        let f = fixture();
        let h = harness(
            &f.exe,
            &f.journal,
            "",
            "pi-sess-42",
            false,
            None,
            Some("/tmp/pi-sessions".into()),
            Duration::from_secs(10),
        )
        .await
        .expect("handshake succeeds");
        assert_eq!(h.handle.session_id, "routing-1");
        assert!(!h.handle.resumed);
        assert_eq!(h.handle.acp_session_id.as_deref(), Some("pi-sess-42"));
        let model = h.handle.model.expect("model surface reported");
        assert_eq!(model.current, "anthropic/m-1");
        assert_eq!(model.options, vec!["anthropic/m-1", "openai/g-5"]);

        let argv = &journal_lines(&f.journal)[0];
        assert!(argv.starts_with("argv: "), "journal logs argv: {argv}");
        assert!(argv.contains("--mode rpc"), "argv: {argv}");
        assert!(argv.contains("--session-dir /tmp/pi-sessions"), "argv: {argv}");
        assert!(
            !argv.split_whitespace().any(|a| a == "--session"),
            "fresh spawn 不带 --session: {argv}"
        );
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// 2.2 全回合流：ThinkingDelta/TextDelta/ToolStart/ToolEnd/UsageUpdate 依
    /// 序到达；agent_end 不收尾、agent_settled 才是唯一 Finished。
    #[tokio::test]
    async fn full_turn_settles_once() {
        let f = fixture();
        let mut h = harness(
            &f.exe,
            &f.journal,
            "",
            "pi-sess-1",
            false,
            None,
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        // 命令面板随握手广播（get_commands 应答 → AvailableCommands）。
        let first = h.evt_rx.recv().await.unwrap();
        assert!(
            matches!(&first, AcpEvent::AvailableCommands { commands, .. } if commands.len() == 1),
            "first event is the command panel: {first:?}"
        );

        h.cmd_tx
            .send(AcpCommand::ContinueSession {
                session_id: "routing-1".into(),
                prompt: "hello".into(),
            })
            .await
            .unwrap();

        let mut seen: Vec<AcpEvent> = Vec::new();
        let mut finished = 0;
        while let Some(evt) = h.evt_rx.recv().await {
            match evt {
                AcpEvent::Finished { .. } => finished += 1,
                AcpEvent::Error { terminal: true, .. } => panic!("unexpected terminal error"),
                other => seen.push(other),
            }
            if finished == 1 {
                break;
            }
        }
        let kind = |e: &AcpEvent| -> &'static str {
            match e {
                AcpEvent::TextDelta { .. } => "text",
                AcpEvent::ThinkingDelta { .. } => "thinking",
                AcpEvent::ToolStart { .. } => "tool_start",
                AcpEvent::ToolEnd { .. } => "tool_end",
                AcpEvent::UsageUpdate { .. } => "usage",
                _ => "other",
            }
        };
        let order: Vec<&str> = seen.iter().map(kind).collect();
        for expected in ["thinking", "text", "tool_start", "tool_end"] {
            assert!(order.contains(&expected), "missing {expected}: {order:?}");
        }
        assert_eq!(
            order.iter().filter(|k| **k == "usage").count(),
            2,
            "两条变化 usage 都到达: {order:?}"
        );
        // 词表纪律：pi 会话永不产生 PermissionRequest。
        assert!(!seen
            .iter()
            .any(|e| matches!(e, AcpEvent::PermissionRequest { .. })));
        assert_eq!(finished, 1, "agent_end 不收尾，agent_settled 才是唯一 Finished");
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// 2.2 取消：abort 下发 → agent_settled(aborted=true) 后才 Finished。
    #[tokio::test]
    async fn cancel_aborts_then_settles_before_finished() {
        let f = fixture();
        let mut h = harness(
            &f.exe,
            &f.journal,
            "",
            "pi-sess-1",
            false,
            None,
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        h.cmd_tx
            .send(AcpCommand::Cancel {
                session_id: "routing-1".into(),
            })
            .await
            .unwrap();

        let mut finished = 0;
        while let Some(evt) = h.evt_rx.recv().await {
            if let AcpEvent::Finished { .. } = evt {
                finished += 1;
                break;
            }
        }
        assert_eq!(finished, 1, "取消以 settled → Finished 收尾");
        assert!(
            journal_lines(&f.journal)
                .iter()
                .any(|l| l.contains(r#""type":"abort""#)),
            "abort 命令确实下发"
        );
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// 2.2 startup timeout 强制执行：二进制在场但挂起 → 超时杀进程、按
    /// DriverError::Timeout 失败（口径与其余 driver 一致）。
    #[tokio::test]
    async fn hanging_binary_times_out() {
        let f = fixture();
        let err = harness(
            &f.exe,
            &f.journal,
            "hang",
            "pi-sess-1",
            false,
            None,
            None,
            Duration::from_secs(1),
        )
        .await
        .err()
        .expect("hang must time out");
        assert!(
            matches!(err, DriverError::Timeout(d) if d == Duration::from_secs(1)),
            "timeout error: {err:?}"
        );
    }

    /// 2.3 恢复被挂接：spawn argv 带 `--session pi-sess-9`，resumed=true，
    /// 握手上报同一 pi 会话 id。
    #[tokio::test]
    async fn resume_attaches_by_session_flag() {
        let f = fixture();
        let h = harness(
            &f.exe,
            &f.journal,
            "",
            "pi-sess-9",
            true,
            Some("pi-sess-9".into()),
            Some("/tmp/pi-sessions".into()),
            Duration::from_secs(10),
        )
        .await
        .expect("resume attaches");
        assert!(h.handle.resumed);
        assert_eq!(h.handle.session_id, "routing-1");
        assert_eq!(h.handle.acp_session_id.as_deref(), Some("pi-sess-9"));
        let argv = &journal_lines(&f.journal)[0];
        assert!(
            argv.split_whitespace().any(|a| a == "--session") && argv.contains("pi-sess-9"),
            "argv carries --session <id>: {argv}"
        );
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// 2.3 恢复被拒诚实回落：fake pi 见 `--session` 即退出 → 驱动回落新会话
    /// （新 routing id、resumed=false），fresh 握手照常完成。
    #[tokio::test]
    async fn rejected_resume_falls_back_to_fresh() {
        let dir = tempfile::tempdir().unwrap();
        // 第二个 fake：任何 --session 参数都拒绝（进程退出，stderr 报原因）。
        let script = format!(
            r#"#!/bin/sh
for a in "$@"; do
  [ "$a" = "--session" ] && {{ echo "session not found" >&2; exit 1; }}
done
{}
"#,
            FAKE_PI
        );
        let exe = shared_script("fake-pi-reject.sh", &script, &FAKE_PI_REJECT_EXE);
        let journal = dir.path().join("journal.log").display().to_string();

        let h = harness(
            &exe,
            &journal,
            "",
            "pi-fresh-2",
            true,
            Some("pi-sess-gone".into()),
            Some("/tmp/pi-sessions".into()),
            Duration::from_secs(10),
        )
        .await
        .expect("fallback fresh succeeds");
        assert!(!h.handle.resumed, "回落会话 resumed=false");
        assert_ne!(h.handle.session_id, "routing-1", "回落换新 routing id");
        assert_eq!(
            h.handle.acp_session_id.as_deref(),
            Some("pi-fresh-2"),
            "回落会话上报 fresh pi 会话 id"
        );
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// set_model 失败如实上报（非终态 + 稳定标记），会话保持可用；成功路径
    /// 发 ModelChanged。
    #[tokio::test]
    async fn set_model_failure_is_honest_and_non_terminal() {
        let f = fixture();
        let mut h = harness(
            &f.exe,
            &f.journal,
            "",
            "pi-sess-1",
            false,
            None,
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        drain_command_panel(&mut h.evt_rx).await;

        h.cmd_tx
            .send(AcpCommand::SetModel {
                session_id: "routing-1".into(),
                model_id: "anthropic/bad".into(),
            })
            .await
            .unwrap();
        match h.evt_rx.recv().await.unwrap() {
            AcpEvent::Error { message, terminal, .. } => {
                assert!(!terminal);
                assert!(message.contains("bad"), "{message}");
                assert!(message.contains(crate::MODEL_UNCHANGED_MARKER), "{message}");
            }
            other => panic!("expected error event, got {other:?}"),
        }

        h.cmd_tx
            .send(AcpCommand::SetModel {
                session_id: "routing-1".into(),
                model_id: "anthropic/m-2".into(),
            })
            .await
            .unwrap();
        match h.evt_rx.recv().await.unwrap() {
            AcpEvent::ModelChanged { model_id, .. } => assert_eq!(model_id, "anthropic/m-2"),
            other => panic!("expected ModelChanged, got {other:?}"),
        }
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// SetMode 得到非终态「不支持」（D4）：会话存活、后续命令照常（看门狗
    /// 周期探针被容忍的口径）。
    #[tokio::test]
    async fn set_mode_answers_unsupported_and_session_lives_on() {
        let f = fixture();
        let mut h = harness(
            &f.exe,
            &f.journal,
            "",
            "pi-sess-1",
            false,
            None,
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        drain_command_panel(&mut h.evt_rx).await;

        h.cmd_tx
            .send(AcpCommand::SetMode {
                session_id: "routing-1".into(),
                mode: "ask".into(),
            })
            .await
            .unwrap();
        match h.evt_rx.recv().await.unwrap() {
            AcpEvent::Error { message, terminal, .. } => {
                assert!(!terminal);
                assert!(message.contains("无权限系统"), "{message}");
            }
            other => panic!("expected error event, got {other:?}"),
        }

        // 会话存活：后续 prompt 照常开回合。
        h.cmd_tx
            .send(AcpCommand::ContinueSession {
                session_id: "routing-1".into(),
                prompt: "still alive".into(),
            })
            .await
            .unwrap();
        let mut finished = false;
        while let Some(evt) = h.evt_rx.recv().await {
            if matches!(evt, AcpEvent::Finished { .. }) {
                finished = true;
                break;
            }
        }
        assert!(finished, "SetMode 后会话仍可对话");
        drop(h.cmd_tx);
        join_run(h.run_task).await;
    }

    /// 进程意外退出 → terminal Error（路由移除映射）。
    #[tokio::test]
    async fn unexpected_exit_is_terminal() {
        let f = fixture();
        let mut h = harness(
            &f.exe,
            &f.journal,
            "die_after_handshake",
            "pi-sess-1",
            false,
            None,
            None,
            Duration::from_secs(10),
        )
        .await
        .unwrap();

        let mut saw_terminal = false;
        while let Some(evt) = h.evt_rx.recv().await {
            if let AcpEvent::Error { terminal, .. } = evt {
                assert!(terminal, "进程退出必须是 terminal Error");
                saw_terminal = true;
                break;
            }
        }
        assert!(saw_terminal);
        join_run(h.run_task).await;
    }

    /// argv 组装（纯函数）：旗标顺序与缺省行为。
    #[test]
    fn argv_assembly_appends_protocol_flags() {
        let argv = assemble_argv(
            &["pi".into(), "--extension".into(), "./x.ts".into()],
            Some("/srv/sessions"),
            None,
        );
        assert_eq!(
            argv,
            vec![
                "pi".to_string(),
                "--extension".into(),
                "./x.ts".into(),
                "--mode".into(),
                "rpc".into(),
                "--session-dir".into(),
                "/srv/sessions".into(),
            ]
        );
        let argv = assemble_argv(&["pi".into()], None, Some("sid-9"));
        assert_eq!(
            argv,
            vec![
                "pi".to_string(),
                "--mode".into(),
                "rpc".into(),
                "--session".into(),
                "sid-9".into(),
            ]
        );
        // 空 sessions_dir 视同未配置（不加 --session-dir）。
        let argv = assemble_argv(&["pi".into()], Some(""), None);
        assert_eq!(argv, vec!["pi".to_string(), "--mode".into(), "rpc".into()]);
    }

    /// 模型选择面组装：无模型无 current → None；current 不在列表时前插。
    #[test]
    fn model_info_is_built_from_the_advertised_surface() {
        assert_eq!(build_model_info(None, &[]), None);
        let info = build_model_info(None, &[("openai".into(), "g-5".into())]).unwrap();
        assert_eq!(info.options, vec!["openai/g-5"]);
        assert_eq!(info.current, "");
        let info = build_model_info(
            Some(&("anthropic".into(), "m-9".into())),
            &[("openai".into(), "g-5".into())],
        )
        .unwrap();
        assert_eq!(info.current, "anthropic/m-9");
        assert_eq!(info.options, vec!["anthropic/m-9", "openai/g-5"]);
    }
}
