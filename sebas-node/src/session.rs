//! 节点侧会话宿主（add-remote-execution-node 3.3 / 4.2 / 4.3 / 4.6）。
//!
//! 职责：把链路来的会话操作落到**节点本地**——校验项目路径、执行并发上限、建立
//! 会话、把执行体的输出写进本地有序日志（[`crate::log`]）、并把日志按时间窗合并成
//! turn 批上报。
//!
//! ## 为什么有 [`ExecutionBody`] 这层抽象
//!
//! 节点要能承载不同执行体（ACP 子进程、原生内核…），而宿主本身只关心「投递输入 →
//! 产出条目 → 取消/关闭/切模型」。把这层抽出来有三个好处：
//!
//! 1. **可测**：`echo` 执行体让宿主逻辑（顺序、合并、拒绝、容量）能在毫秒级单测里
//!    被完整覆盖，不必起真子进程；
//! 2. **诚实**：执行体做不到的能力（如无法强制 mode）由它自己回报，宿主不替它编造；
//! 3. **可演进**：接 ACP 子进程时只加一个实现，宿主与协议都不动。
//!
//! ## 合并不等于丢弃
//!
//! 传输层按时间窗合并（4.2），但**日志里的原始序列永远完整**——控制面按 seq 回拉
//! 即得精确内容（4.3）。缓冲超限时批次带 `coalesced_overflow` 标记，绝不静默丢。

use crate::log::SessionLog;
use sebas_node_link::{
    ApprovalDecision, GateCategory, LogEntry, ParkedApproval, SessionEvent, SessionMode,
    SessionRejectCode, SessionSummary,
};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 执行体产出的一个条目（尚未分配 seq）。
#[derive(Debug, Clone, PartialEq)]
pub struct BodyEvent {
    /// 条目类型（写进 [`LogEntry::kind`]）。
    pub kind: String,
    /// 文本内容。
    pub text: String,
    /// 结构化附加信息。
    pub data: Option<serde_json::Value>,
    /// 本轮想做的**受门控动作**（`Some` 即需要宿主按 mode 判定）。
    ///
    /// 真实执行体（ACP 子进程）在接到 agent 的权限请求时填这里；宿主据此决定
    /// 「按模式放行」还是「上报控制面并停住」。
    pub gate: Option<GateRequest>,
}

/// 一次受门控动作的请求（来自执行体）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRequest {
    /// 工具/动作名。
    pub tool: String,
    /// 动作类别。
    pub category: GateCategory,
    /// 执行体自己的**回执句柄**：宿主拿到控制面的决定后，靠它把结论送回这个
    /// 正在等着的执行体（如 ACP 的 `request_id` / 权限 oneshot 的键）。
    ///
    /// `None` = 这个执行体不需要回执（`echo` 只把门报上来就返回了）；`Some` = 宿主
    /// 在放行或决议后**必须**回调 [`ExecutionBody::resolve_gate`]，否则执行体一直
    /// 停在门上——「节点不自我放行」不等于「节点把批准过的动作卡死」。
    pub resume_token: Option<String>,
}

/// 节点上真正干活的执行体。
///
/// 同步形态：`prompt` 把本轮产出写进 `out`。真实子进程实现（ACP）在内部泵事件并
/// 在同一调用里收集，另外用 [`ExecutionBody::drain`] 把审批之后继续产出的条目交出来
/// ——宿主不需要感知底下的任务/线程。
pub trait ExecutionBody: Send {
    /// 实际生效的执行体 kind。
    fn agent_kind(&self) -> String;

    /// 实际生效的模型（无模型概念 → `None`）。
    fn model(&self) -> Option<String>;

    /// 实际生效的 mode（无法强制 → `None`；宿主据此回报 effective 值）。
    fn mode(&self) -> Option<String>;

    /// 本执行体能否**强制**会话 mode。
    ///
    /// `true` → 宿主回报 `effective = desired`；`false` → 宿主如实回报「没有可声称
    /// 生效的 mode」（`mode: None`），同时仍按期望模式尽力门控它**能看见**的请求。
    /// 缺省 `true`：宿主自己就在门控环路上（`echo` 即此类）。
    fn enforces_mode(&self) -> bool {
        true
    }

    /// 投递一轮输入。
    fn prompt(&mut self, text: &str, out: &mut Vec<BodyEvent>) -> Result<(), String>;

    /// 取消在飞 turn；返回是否确有在飞的 turn 被取消。
    fn cancel(&mut self) -> Result<bool, String>;

    /// 期望模型变更；`Ok(effective)` 里是实际生效值。
    fn set_model(&mut self, model: &str) -> Result<Option<String>, String>;

    /// 关闭（终止子进程等）。可重复调用。
    fn close(&mut self);

    /// 把门控结论送回停在门上的执行体（按 [`GateRequest::resume_token`] 关联）。
    ///
    /// 只有 [`SessionHost::answer_approval`] 与模式自动放行会调用它——节点侧没有
    /// 任何**别的**入口能给出结论（6.6 的安全性质）。缺省实现如实回错：该执行体
    /// 不上报 `resume_token`，就不该被回调。
    fn resolve_gate(&mut self, resume_token: &str, _decision: ApprovalDecision) -> Result<(), String> {
        Err(format!(
            "执行体 {} 不支持门控回执（resume_token {resume_token:?} 无处安放）",
            self.agent_kind()
        ))
    }

    /// 取出**已产生但尚未上报**的条目（如审批通过后 agent 继续跑出来的输出）。
    ///
    /// 契约：**取到第一个 `gate` 为止（含）**。宿主遇到门就会停住本轮并 park，
    /// 若执行体一次把门之后的条目也交出来，那些「还没获准就可能已经发生」的条目
    /// 会被记进日志——所以门之后的内容必须留在执行体里，等决议后再交。
    fn drain(&mut self, _out: &mut Vec<BodyEvent>) -> Result<(), String> {
        Ok(())
    }
}

impl HostedSession {
    /// 取执行体；已关闭 → `SessionClosed`（会话还在、日志还在，只是不再收输入）。
    fn body_mut(&mut self, session_id: &str) -> Result<&mut Box<dyn ExecutionBody>, Rejected> {
        self.body.as_mut().ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::SessionClosed,
                format!("会话 {session_id} 已关闭：日志仍可查询，但不再接受输入"),
            )
        })
    }
}

/// 节点侧的会话宿主。
pub struct SessionHost {
    log_dir: PathBuf,
    max_sessions: usize,
    sessions: HashMap<String, HostedSession>,
    /// 每会话待上报的批次。
    pending: HashMap<String, PendingBatch>,
    /// 非批事件的外发队列（审批请求 / 门控结论 / 审计提示）。
    outbox: VecDeque<SessionEvent>,
    /// 下一次 spawn 要钉住的材料版本与落点。
    ///
    /// 材料拉取是异步的（要走链路），而宿主是同步的：由链路层先拉好、再放进这个槽，
    /// 宿主建会话时**取走**并记进日志。取走语义（而不是读取）保证它不会漏到下一个
    /// 会话上——那样会让新会话"继承"上一个会话的材料版本。
    pending_materials: Option<(String, PathBuf)>,
    /// 合并时间窗。
    coalesce_window: Duration,
    /// 单批条目上限（超出即打 `coalesced_overflow` 标记，内容仍在日志里）。
    coalesce_max_entries: usize,
    /// 存储上限（字节）；0 表示不限。
    storage_ceiling_bytes: u64,
    factory: Arc<dyn BodyFactory + Send + Sync>,
}

struct HostedSession {
    log: SessionLog,
    /// 控制面持有的期望模式。**门控按它尽力执行**：执行体能不能保证是另一回事
    /// （见 `effective_mode`）。
    desired_mode: SessionMode,
    /// 实际生效模式（执行体能否强制）；无法强制 → `None`（不把期望值回显成已生效）。
    effective_mode: Option<SessionMode>,
    /// 期望的 provider profile。
    desired_provider: Option<String>,
    /// 实际生效的 provider。
    provider: Option<String>,
    /// 期望与实际不同时的成因。
    provider_cause: Option<String>,
    /// **悬空**的审批请求。节点侧没有任何本地裁决路径：只有控制面的决定能把它们
    /// 移出这里（不存在超时、不存在自动拒绝、更不存在自动放行）。
    parked: Vec<ParkedApproval>,
    /// 宿主请求 id → 执行体自己的回执句柄（见 [`GateRequest::resume_token`]）。
    parked_tokens: HashMap<String, String>,
    /// 审批请求 id 的会话内序号。
    next_request: u64,
    /// 钉住的操作者级材料版本（会话创建时确定，此后不跟随更新）。
    materials_version: Option<String>,
    /// 执行体；会话关闭后为 `None`——**日志仍然保留并可查询**（执行事实不因关闭
    /// 消失，否则最后几轮 turn 可能在与 close 的竞态里永远到不了控制面）。
    body: Option<Box<dyn ExecutionBody>>,
    phase: String,
}

struct PendingBatch {
    from_seq: u64,
    entries: Vec<LogEntry>,
    first_at: Instant,
    /// 是否发生过「条目超出单批上限」的压缩（内容仍可回拉）。
    overflow: bool,
}

/// 建会话时交给工厂的期望值（期望 → 由工厂/执行体如实回报实际值）。
#[derive(Debug, Clone)]
pub struct BodySpec<'a> {
    /// 期望的执行体 kind。
    pub kind: &'a str,
    /// 项目目录（在**节点**上的一份路径；`None` = 无项目会话）。
    pub project_dir: Option<&'a Path>,
    /// 期望模型。
    pub model: Option<&'a str>,
    /// 期望 mode。
    pub mode: Option<&'a str>,
    /// 期望的节点本地 provider profile 名（7.1）。
    pub provider: Option<&'a str>,
}

/// 工厂的产物：执行体 + provider 归属（desired 进、effective 出 + 成因）。
pub struct MadeBody {
    /// 真正干活的执行体。
    pub body: Box<dyn ExecutionBody>,
    /// 实际生效的 provider（`control-plane-router` 或 profile 名；未选 → `None`）。
    pub provider: Option<String>,
    /// 控制面期望的 provider（节点未收到期望时为 `None`，即使应用了默认 profile）。
    pub desired_provider: Option<String>,
    /// 期望与实际不同时的成因（相同 → `None`）。
    pub provider_cause: Option<String>,
}

impl MadeBody {
    /// 不涉及 provider 的执行体（`echo`）。
    pub fn plain(body: Box<dyn ExecutionBody>) -> Self {
        Self {
            body,
            provider: None,
            desired_provider: None,
            provider_cause: None,
        }
    }
}

impl std::fmt::Debug for MadeBody {
    /// 手写 Debug：执行体本身不实现 Debug，但测试要在 `unwrap_err()` 里打印
    /// 「成功侧」的 provider 归属。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MadeBody")
            .field("agent_kind", &self.body.agent_kind())
            .field("provider", &self.provider)
            .field("desired_provider", &self.desired_provider)
            .field("provider_cause", &self.provider_cause)
            .finish()
    }
}

/// 执行体工厂：按 kind 造执行体，或如实回「不支持 / 不可用」。
pub trait BodyFactory: Send + Sync {
    /// 建立执行体。`Err` 是带码的如实拒绝（宿主原样透传，不再改写语义）。
    fn make(&self, spec: &BodySpec<'_>) -> Result<MadeBody, Rejected>;

    /// 本节点可服务的 kind 列表（能力清单用）。
    fn available_kinds(&self) -> Vec<String>;
}

/// 立即产出一个条目的执行体：用于测试与「无 agent 的连通性验证」。
///
/// 它**不假装**自己是模型：`agent_kind` 明确回报 `echo`，模型与 mode 如实回报。
#[derive(Debug, Default)]
pub struct EchoBody {
    model: Option<String>,
    mode: Option<String>,
    closed: bool,
    cancelled: u32,
}

impl ExecutionBody for EchoBody {
    fn agent_kind(&self) -> String {
        "echo".into()
    }

    fn model(&self) -> Option<String> {
        self.model.clone()
    }

    fn mode(&self) -> Option<String> {
        self.mode.clone()
    }

    fn prompt(&mut self, text: &str, out: &mut Vec<BodyEvent>) -> Result<(), String> {
        if self.closed {
            return Err("会话已关闭".into());
        }
        out.push(BodyEvent {
            kind: "prompt".into(),
            text: text.to_string(),
            data: None,
            gate: None,
        });
        // 约定：以 `run:` 开头的输入代表一次受门控的命令执行（测试与连通性验证用）。
        // 真实执行体在收到 agent 的权限请求时做同样的事。
        if let Some(command) = text.strip_prefix("run:") {
            out.push(BodyEvent {
                kind: "gate".into(),
                text: format!("requested: {}", command.trim()),
                data: None,
                gate: Some(GateRequest {
                    tool: "bash".into(),
                    category: GateCategory::Execute,
                    // echo 同步返回、不停在门上，因此不需要回执。
                    resume_token: None,
                }),
            });
            return Ok(());
        }
        out.push(BodyEvent {
            kind: "output".into(),
            text: format!("echo: {text}"),
            data: None,
            gate: None,
        });
        Ok(())
    }

    fn cancel(&mut self) -> Result<bool, String> {
        // echo 是同步的：没有在飞 turn 可取消，如实回报 false。
        self.cancelled += 1;
        Ok(false)
    }

    fn set_model(&mut self, model: &str) -> Result<Option<String>, String> {
        self.model = Some(model.to_string());
        Ok(self.model.clone())
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

/// 只认 `echo` 的工厂（缺省；真实 ACP 执行体见 [`crate::body`]）。
#[derive(Debug, Default)]
pub struct EchoOnlyFactory;

impl BodyFactory for EchoOnlyFactory {
    fn make(&self, spec: &BodySpec<'_>) -> Result<MadeBody, Rejected> {
        if spec.kind != "echo" && !spec.kind.is_empty() {
            return Err(Rejected::new(
                SessionRejectCode::UnsupportedAgentKind,
                format!("本节点未配置执行体 kind {:?}", spec.kind),
            ));
        }
        Ok(MadeBody::plain(Box::new(EchoBody {
            model: spec.model.map(|m| m.to_string()),
            mode: spec.mode.map(|m| m.to_string()),
            ..EchoBody::default()
        })))
    }

    fn available_kinds(&self) -> Vec<String> {
        vec!["echo".into()]
    }
}

/// 拒绝：机器可判别的码 + 人可读成因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// 拒绝码。
    pub code: SessionRejectCode,
    /// 成因。
    pub cause: String,
}

impl Rejected {
    pub(crate) fn new(code: SessionRejectCode, cause: impl Into<String>) -> Self {
        Self {
            code,
            cause: cause.into(),
        }
    }
}

/// `spawn` 成功后的实际生效值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    /// 日志纪元。
    pub epoch: u64,
    /// 本会话钉住的操作者级材料版本（未使用材料 → `None`）。
    pub materials_version: Option<String>,
    /// 实际生效的执行体 kind。
    pub agent_kind: String,
    /// 实际生效的模型。
    pub model: Option<String>,
    /// 实际生效的 mode（强制不了 → `None`）。
    pub mode: Option<String>,
    /// 实际生效的 provider（`control-plane-router` 或 profile 名；未选 → `None`）。
    pub provider: Option<String>,
    /// provider 期望与实际不同时的成因（相同 → `None`）。
    pub provider_cause: Option<String>,
}

impl SessionHost {
    /// 以日志目录、并发上限与执行体工厂构造。
    ///
    /// **会重新挂上磁盘上的孤立日志**：节点重启后内存里的会话全没了，但日志还在。
    /// 若不挂回去，那些「执行事实」在协议上就再也拉不到（`UnknownSession`），
    /// 而它们其实好好躺在磁盘上。挂回来的会话相位为 `terminated`：身份仍在、日志可查、
    /// 不接受输入（要重新工作请重新 spawn）——节点重启**终止**会话，但不销毁事实。
    pub fn new(
        log_dir: impl Into<PathBuf>,
        max_sessions: usize,
        storage_ceiling_bytes: u64,
        factory: Arc<dyn BodyFactory + Send + Sync>,
    ) -> Self {
        let mut host = Self {
            log_dir: log_dir.into(),
            max_sessions,
            sessions: HashMap::new(),
            pending: HashMap::new(),
            outbox: VecDeque::new(),
            pending_materials: None,
            coalesce_window: Duration::from_millis(150),
            coalesce_max_entries: 256,
            storage_ceiling_bytes,
            factory,
        };
        host.attach_orphan_logs();
        host
    }

    /// 把磁盘上有日志、内存里却没有的会话挂回来（见 [`Self::new`]）。
    fn attach_orphan_logs(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.log_dir) else {
            return;
        };
        // 认 `*.log.jsonl` 而不是 `*.meta.json`：meta 只在**回收或重置**时才写，
        // 于是普通会话（从未回收过）根本没有 meta 文件——按 meta 扫描会让重启后
        // 一个日志都挂不回来（这正是集成测试抓到的问题）。
        let mut ids: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_suffix(".log.jsonl").map(str::to_string)
            })
            .collect();
        ids.sort();
        for session_id in ids {
            if self.sessions.contains_key(&session_id) {
                continue;
            }
            match SessionLog::open(&self.log_dir, &session_id) {
                Ok(log) => {
                    self.sessions.insert(
                        session_id.clone(),
                        HostedSession {
                            log,
                            desired_mode: SessionMode::Ask,
                            effective_mode: Some(SessionMode::Ask),
                            desired_provider: None,
                            provider: None,
                            provider_cause: None,
                            parked: Vec::new(),
                            parked_tokens: HashMap::new(),
                            next_request: 0,
                            materials_version: None,
                            body: None,
                            phase: "terminated".into(),
                        },
                    );
                }
                Err(e) => eprintln!("warning: 无法恢复会话日志 {session_id}：{e}"),
            }
        }
    }

    /// 覆盖合并调参（测试用）。
    pub fn with_coalescing(mut self, window: Duration, max_entries: usize) -> Self {
        self.coalesce_window = window;
        self.coalesce_max_entries = max_entries;
        self
    }

    /// 本节点可服务的执行体 kind（能力清单用）。
    pub fn available_kinds(&self) -> Vec<String> {
        self.factory.available_kinds()
    }

    /// 换掉执行体工厂（链路装配真实工厂时调用；缺省是只认 `echo` 的工厂）。
    ///
    /// 只影响**此后**建立的会话：已经跑着的执行体不会被替换（换掉等于把它手里
    /// 的子进程丢掉）。
    pub fn set_factory(&mut self, factory: Arc<dyn BodyFactory + Send + Sync>) {
        self.factory = factory;
    }

    /// 指定**下一次** spawn 要钉住的材料版本与落点（由链路层拉取后调用）。
    ///
    /// 取走语义：宿主建会话时把它移走，绝不会漏到下一个会话。
    pub fn set_pending_materials(&mut self, version: impl Into<String>, dir: impl Into<PathBuf>) {
        self.pending_materials = Some((version.into(), dir.into()));
    }

    /// 建立会话。
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &mut self,
        session_id: &str,
        project_dir: Option<&str>,
        agent_kind: Option<&str>,
        model: Option<&str>,
        mode: Option<&str>,
        provider: Option<&str>,
    ) -> Result<Spawned, Rejected> {
        if self.sessions.contains_key(session_id) {
            return Err(Rejected::new(
                SessionRejectCode::DuplicateSession,
                format!("会话 {session_id} 已存在；重新建立前先 close"),
            ));
        }
        // 容量只数**活**会话：已关闭的会话保留日志但不再占并发额度（4.6）。
        let live = self.sessions.values().filter(|s| s.body.is_some()).count();
        if live >= self.max_sessions {
            return Err(Rejected::new(
                SessionRejectCode::OverCapacity,
                format!("本节点并发上限 {} 已满", self.max_sessions),
            ));
        }
        // 项目路径在**真正 spawn 的这台机器**上判定（3.3）。
        let project = match project_dir {
            Some(dir) => {
                let path = PathBuf::from(dir);
                if !path.is_dir() {
                    return Err(Rejected::new(
                        SessionRejectCode::UnusableProjectDir,
                        format!("{} 在本节点上不是可用目录", path.display()),
                    ));
                }
                Some(path)
            }
            None => None,
        };
        // 存储上限：触顶时如实拒绝新会话，而不是丢历史（4.5）。
        if self.storage_ceiling_bytes > 0
            && self.disk_usage() >= self.storage_ceiling_bytes
        {
            return Err(Rejected::new(
                SessionRejectCode::StorageExhausted,
                format!(
                    "本节点存储已达上限 {} 字节（既有历史不被丢弃）",
                    self.storage_ceiling_bytes
                ),
            ));
        }

        // 模式：缺省 `ask`（`auto` **永远不是**缺省），不认识的模式如实拒绝而不是
        // 悄悄降级——把 "yolo" 当成 ask 会让人以为管控生效了。
        let desired_mode = match mode {
            Some(raw) => SessionMode::parse(raw).ok_or_else(|| {
                Rejected::new(
                    SessionRejectCode::UnsupportedMode,
                    format!("不认识的会话模式 {raw:?}（可用：ask / edit / allow / auto）"),
                )
            })?,
            None => SessionMode::Ask,
        };

        let kind = agent_kind.unwrap_or("echo");
        let made = self
            .factory
            .make(&BodySpec {
                kind,
                project_dir: project.as_deref(),
                model,
                mode,
                provider,
            })
            .map_err(|r| r)?;
        let MadeBody {
            body,
            provider: effective_provider,
            desired_provider,
            provider_cause,
        } = made;
        // 材料版本与落点：**取走**（见 pending_materials 的文档）。
        let materials = self.pending_materials.take();
        // 实际生效模式：能强制 → 执行体回报的值（缺省与期望一致）；不能强制 →
        // 如实回「没有可声称生效的 mode」，把差异留给控制面呈现（设计 D6）。
        let effective_mode = if body.enforces_mode() {
            Some(
                body.mode()
                    .and_then(|s| SessionMode::parse(&s))
                    .unwrap_or(desired_mode),
            )
        } else {
            None
        };

        let mut log = SessionLog::open(&self.log_dir, session_id)
            .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))?;
        log.append("state", "spawning", None)
            .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))?;
        log.append("state", "active", None)
            .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))?;

        // 钉住材料版本并留痕（可复现性：事后要能回答"这个会话当时用的是哪一版"）。
        if let Some((version, dir)) = &materials {
            let _ = log.append(
                "materials_pinned",
                &format!("材料版本 {version} → {}", dir.display()),
                Some(serde_json::json!({ "version": version, "dir": dir.display().to_string() })),
            );
        }
        let spawned = Spawned {
            epoch: log.epoch(),
            agent_kind: body.agent_kind(),
            model: body.model(),
            mode: effective_mode.map(|m| m.as_str().to_string()),
            provider: effective_provider.clone(),
            provider_cause: provider_cause.clone(),
            materials_version: materials.as_ref().map(|(v, _)| v.clone()),
        };
        self.sessions.insert(
            session_id.to_string(),
            HostedSession {
                log,
                desired_mode,
                effective_mode,
                desired_provider,
                provider: effective_provider,
                provider_cause,
                parked: Vec::new(),
                parked_tokens: HashMap::new(),
                next_request: 0,
                materials_version: materials.as_ref().map(|(v, _)| v.clone()),
                body: Some(body),
                phase: "active".into(),
            },
        );
        Ok(spawned)
    }

    /// 投递一轮输入。
    ///
    /// 逐条落账；**遇到受门控动作就按模式放行或停驻**，并且停在那里——审批之后、
    /// 审批之前的动作本来就没有发生，继续往下写会把没发生的事记成发生过。
    pub fn prompt(&mut self, session_id: &str, text: &str) -> Result<(), Rejected> {
        let body_events = {
            let session = self.session_mut(session_id)?;
            let mut events = Vec::new();
            session
                .body_mut(session_id)?
                .prompt(text, &mut events)
                .map_err(|cause| Rejected::new(SessionRejectCode::NodeError, cause))?;
            events
        };
        self.record_body_events(session_id, body_events)
    }

    /// 把执行体产出的条目落账（`prompt` 与 [`Self::drain_body`] 共用）。
    ///
    /// 三条不变量：
    /// 1. **遇到门就停**：门之后的条目还没获准发生，落账等于把没发生的事记成发生过；
    ///    （执行体的 `drain` 契约只交到门为止，所以这里 break 不会丢条目。）
    /// 2. **执行体死亡如实终结会话**：终态错误不再是「一次失败的本轮」，而是子进程
    ///    没了——相位转 `exited`、上报 `Exited`，日志保留可查。
    /// 3. 门控结论（自动放行）**必须回头喂给执行体**，否则停在门上的 agent 永远等下去。
    fn record_body_events(
        &mut self,
        session_id: &str,
        body_events: Vec<BodyEvent>,
    ) -> Result<(), Rejected> {
        let mut recorded: Vec<LogEntry> = Vec::new();
        let mut terminal: Option<String> = None;
        for event in body_events {
            if event.kind == "error"
                && event
                    .data
                    .as_ref()
                    .and_then(|d| d.get("terminal"))
                    .and_then(|v| v.as_bool())
                    == Some(true)
            {
                terminal = Some(event.text.clone());
            }
            match event.gate.clone() {
                Some(gate) => {
                    self.record_gate(session_id, &gate, &mut recorded)?;
                    // 门控之后不再处理：turn 停在这里。
                    break;
                }
                None => {
                    let entry = {
                        let session = self.session_mut(session_id)?;
                        session
                            .log
                            .append_entry(&event.kind, &event.text, event.data.clone())
                            .map_err(|e| {
                                Rejected::new(SessionRejectCode::NodeError, e.to_string())
                            })?
                    };
                    recorded.push(entry);
                }
            }
        }

        if !recorded.is_empty() {
            self.enqueue_batch(session_id, &recorded);
        }
        if let Some(cause) = terminal {
            self.mark_exited(session_id, &cause)?;
        }
        Ok(())
    }

    /// 抽出执行体在审批之后继续产出的条目（真实 ACP 执行体的输出不会恰好在本轮
    /// `prompt` 里到齐：agent 是先停在门上、获准后才继续跑的）。
    ///
    /// 会话已终结 / 不存在 → 静默跳过（巡检路径不该因为一条陈旧记录报错）。
    fn drain_body(&mut self, session_id: &str) {
        let events = {
            let Some(session) = self.sessions.get_mut(session_id) else {
                return;
            };
            let Some(body) = session.body.as_mut() else {
                return;
            };
            let mut events = Vec::new();
            if let Err(e) = body.drain(&mut events) {
                eprintln!("sebas-node: 会话 {session_id} 拉取执行体输出失败：{e}");
                return;
            }
            events
        };
        if events.is_empty() {
            return;
        }
        if let Err(r) = self.record_body_events(session_id, events) {
            eprintln!(
                "sebas-node: 会话 {session_id} 落账执行体输出失败（{}）：{}",
                r.code.as_str(),
                r.cause
            );
        }
    }

    /// 会话终结（子进程死亡）：相位转 `exited`、丢开执行体、上报结论，**日志保留**。
    fn mark_exited(&mut self, session_id: &str, cause: &str) -> Result<(), Rejected> {
        let session = self.session_mut(session_id)?;
        if session.body.is_none() {
            return Ok(());
        }
        session.body = None;
        session.phase = "exited".into();
        let _ = session
            .log
            .append("state", "exited", Some(serde_json::json!({ "cause": cause })));
        self.outbox.push_back(SessionEvent::Exited {
            session_id: session_id.to_string(),
            cause: cause.to_string(),
        });
        Ok(())
    }

    /// 门控判定：按模式放行（留审计、并回执给执行体）或停驻等控制面决定。
    fn record_gate(
        &mut self,
        session_id: &str,
        gate: &GateRequest,
        recorded: &mut Vec<LogEntry>,
    ) -> Result<(), Rejected> {
        // 门控按**期望**模式尽力执行：执行体强制不了时（`effective_mode == None`）
        // 宿主仍把期望应用在它看得见的请求上，但不声称这等于强制。
        let (mode, request_id) = {
            let session = self.session_mut(session_id)?;
            session.next_request += 1;
            (
                session.desired_mode,
                format!("{session_id}:req-{}", session.next_request),
            )
        };

        if mode.allows_without_asking(gate.category) {
            // 模式放行：**必须留审计**——事后要能回答「这条命令是谁放行的」。
            let audit_text = format!(
                "mode={} 放行 {}（类别 {:?}）",
                mode.as_str(),
                gate.tool,
                gate.category
            );
            let entry = {
                let session = self.session_mut(session_id)?;
                session
                    .log
                    .append_entry("audit", &audit_text, None)
                    .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))?
            };
            recorded.push(entry);
            // 回执：执行体正停在门上等这一句，不回它 agent 会一直卡着。
            if let Some(token) = gate.resume_token.clone() {
                let resolved = {
                    let session = self.session_mut(session_id)?;
                    match session.body.as_mut() {
                        Some(body) => body.resolve_gate(&token, ApprovalDecision::AllowOnce),
                        // 会话已关闭：没有执行体可回执，但不假装送达。
                        None => Ok(()),
                    }
                };
                if let Err(e) = resolved {
                    let session = self.session_mut(session_id)?;
                    let _ = session.log.append(
                        "error",
                        &format!("模式放行未能送达执行体（{token}）：{e}"),
                        None,
                    );
                }
            }
            self.outbox.push_back(SessionEvent::GateResolved {
                session_id: session_id.to_string(),
                request_id,
                decision: "auto_allowed".into(),
                source: format!("mode:{}", mode.as_str()),
            });
            return Ok(());
        }

        // 需要控制面决定：登记 + 落账 + 上报，然后**停住**（无超时、无自动拒绝）。
        let approval = ParkedApproval {
            session_id: session_id.to_string(),
            request_id: request_id.clone(),
            tool: gate.tool.clone(),
            category: gate.category,
            mode,
        };
        let entry = {
            let session = self.session_mut(session_id)?;
            session.parked.push(approval.clone());
            // 记下执行体的回执句柄：决定回来时要把它送回停在门上的 agent。
            if let Some(token) = &gate.resume_token {
                session
                    .parked_tokens
                    .insert(request_id.clone(), token.clone());
            }
            session
                .log
                .append_entry(
                    "approval_requested",
                    &format!("{}（类别 {:?}，模式 {}）", gate.tool, gate.category, mode.as_str()),
                    Some(serde_json::json!({ "request_id": request_id, "tool": gate.tool })),
                )
                .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))?
        };
        recorded.push(entry);
        self.outbox.push_back(SessionEvent::ApprovalRequested {
            session_id: session_id.to_string(),
            request_id,
            tool: gate.tool.clone(),
            category: gate.category,
            mode,
        });
        Ok(())
    }

    /// 把条目入队成待上报的批（传输层按窗口合并）。
    fn enqueue_batch(&mut self, session_id: &str, recorded: &[LogEntry]) {
        if let Some(pending) = self.pending.get_mut(session_id) {
            Self::push_pending(pending, recorded, self.coalesce_max_entries);
        } else {
            let mut pending = PendingBatch {
                from_seq: recorded.first().map(|e| e.seq).unwrap_or(1),
                entries: Vec::new(),
                first_at: std::time::Instant::now(),
                overflow: false,
            };
            Self::push_pending(&mut pending, recorded, self.coalesce_max_entries);
            self.pending.insert(session_id.to_string(), pending);
        }
    }

    /// 回填一个审批决定（**唯一的裁决入口**：只有它能改变停驻状态）。
    ///
    /// - 未知 `request_id`（含已决议过的迟到决定）→ 可判别拒绝，无副作用；
    /// - 会话已关闭 → 决定**被丢弃**（`applied=false`）并在日志留一条提示；
    /// - 决定送不进执行体 → 同样**如实回报未生效**（不假装批准了）；
    /// - 成功 → 决议落审计、移出停驻队列，并把结论回执给停在门上的执行体。
    pub fn answer_approval(
        &mut self,
        session_id: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<bool, Rejected> {
        let session = self.session_mut(session_id)?;

        // 会话已关闭：丢弃决定并留痕（不静默、也不撒谎说已生效）。
        if session.body.is_none() {
            let _ = session.log.append(
                "audit",
                &format!("丢弃迟到决定 {request_id}：会话已关闭"),
                Some(serde_json::json!({ "decision": decision.as_str() })),
            );
            return Ok(false);
        }

        let Some(position) = session
            .parked
            .iter()
            .position(|p| p.request_id == request_id)
        else {
            return Err(Rejected::new(
                SessionRejectCode::UnknownApprovalRequest,
                format!("审批请求 {request_id} 未知（可能已被决议，或 id 打错）"),
            ));
        };
        session.parked.remove(position);
        let resume_token = session.parked_tokens.remove(request_id);

        // 把决定送回停在门上的执行体；送不到就**不声称已生效**——「点过允许却没
        // 生效」必须能被看见（6.7 的迟到/丢弃语义）。
        let delivered = match (resume_token, session.body.as_mut()) {
            (Some(token), Some(body)) => body.resolve_gate(&token, decision),
            (Some(token), None) => Err(format!("会话已关闭：决定无处送达（{token}）")),
            // 该执行体不上报回执（如 echo）：决定只改变宿主的停驻状态。
            (None, _) => Ok(()),
        };
        if let Err(e) = delivered {
            let _ = session.log.append(
                "error",
                &format!("审批决定 {request_id} 未能送达执行体：{e}"),
                Some(serde_json::json!({ "decision": decision.as_str() })),
            );
            return Ok(false);
        }

        let _ = session.log.append(
            "audit",
            &format!("审批 {request_id} → {}", decision.as_str()),
            Some(serde_json::json!({
                "decision": decision.as_str(),
                "source": "control-plane",
            })),
        );
        self.outbox.push_back(SessionEvent::GateResolved {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            decision: decision.as_str().to_string(),
            source: "control-plane".into(),
        });
        Ok(true)
    }

    /// 当前**悬空**的审批请求（对账用：主控回来时要能全部看见）。
    pub fn parked_approvals(&self) -> Vec<ParkedApproval> {
        let mut all: Vec<ParkedApproval> = self
            .sessions
            .values()
            .flat_map(|s| s.parked.iter().cloned())
            .collect();
        all.sort_by(|a, b| a.request_id.cmp(&b.request_id));
        all
    }

    /// 期望模式变更；返回**实际生效值**（不认识 → 如实拒绝，不降级）。
    ///
    /// 执行体强制不了 mode 时回 `None`——「没有可声称生效的 mode」是诚实的答案，
    /// 把期望值回显成已生效才是撒谎（设计 D6）。
    pub fn set_mode(&mut self, session_id: &str, mode: &str) -> Result<Option<SessionMode>, Rejected> {
        let parsed = SessionMode::parse(mode).ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::UnsupportedMode,
                format!("不认识的会话模式 {mode:?}（可用：ask / edit / allow / auto）"),
            )
        })?;
        let session = self.session_mut(session_id)?;
        if session.body.is_none() {
            return Err(Rejected::new(
                SessionRejectCode::SessionClosed,
                format!("会话 {session_id} 已关闭：模式不可变更"),
            ));
        }
        session.desired_mode = parsed;
        session.effective_mode = if session
            .body
            .as_ref()
            .map(|b| b.enforces_mode())
            .unwrap_or(false)
        {
            Some(parsed)
        } else {
            None
        };
        // `auto` 是「把机器交给 agent」的选择，必须留审计痕迹。
        if parsed.is_ungated() {
            let _ = session.log.append(
                "audit",
                "mode=auto 由控制面开启（此后不再产生审批请求）",
                Some(serde_json::json!({ "source": "control-plane" })),
            );
        }
        Ok(session.effective_mode)
    }

    /// 取消在飞 turn。
    pub fn cancel(&mut self, session_id: &str) -> Result<bool, Rejected> {
        let session = self.session_mut(session_id)?;
        session
            .body_mut(session_id)?
            .cancel()
            .map_err(|cause| Rejected::new(SessionRejectCode::NodeError, cause))
    }

    /// 关闭会话。
    pub fn close(&mut self, session_id: &str) -> Result<(), Rejected> {
        let session = self.session_mut(session_id)?;
        if let Some(mut body) = session.body.take() {
            body.close();
            session.phase = "closed".into();
            session
                .log
                .append("state", "closed", None)
                .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))?;
        }
        // 幂等：重复 close 不报错（关一个已关的会话不是错误）。
        self.pending.remove(session_id);
        Ok(())
    }

    /// 期望模型变更，返回实际生效值。
    pub fn set_model(&mut self, session_id: &str, model: &str) -> Result<Option<String>, Rejected> {
        let session = self.session_mut(session_id)?;
        let effective = session
            .body_mut(session_id)?
            .set_model(model)
            .map_err(|cause| Rejected::new(SessionRejectCode::NodeError, cause))?;
        Ok(effective)
    }

    /// 按 seq 回拉精确序列（4.3）。
    pub fn log_from(
        &self,
        session_id: &str,
        from_seq: u64,
    ) -> Result<(u64, Vec<LogEntry>, u64), Rejected> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::UnknownSession,
                format!("会话 {session_id} 不存在"),
            )
        })?;
        Ok((
            session.log.epoch(),
            session.log.since(from_seq),
            session.log.last_seq(),
        ))
    }

    /// 全部会话摘要（对账用）。
    pub fn list(&self) -> Vec<SessionSummary> {
        let mut out: Vec<SessionSummary> = self
            .sessions
            .iter()
            .map(|(id, s)| self.summary_of(id, s))
            .collect();
        out.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        out
    }

    /// 单会话快照（对账用）。
    pub fn snapshot(
        &self,
        session_id: &str,
    ) -> Result<(SessionSummary, u64, u64, u64), Rejected> {
        let session = self.sessions.get(session_id).ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::UnknownSession,
                format!("会话 {session_id} 不存在"),
            )
        })?;
        Ok((
            self.summary_of(session_id, session),
            session.log.epoch(),
            session.log.last_seq(),
            session.log.reclaimed_through(),
        ))
    }

    /// **保留期回收**（4.4 / 设计 r2）：节点自主把超过保留期的日志前缀回收掉，
    /// 并把推进后的水位线作为 [`SessionEvent::Reclaimed`] 放进 outbox 上报。
    ///
    /// 三件事值得说清：
    ///
    /// 1. **自主**：保留期是**节点自己的**策略（`log_retention_days` 配在节点侧），
    ///    不是主控下令的清理。控制面只被告知结果——所以这个方法不查链路是否在线。
    /// 2. **离线也回收，水位线不丢**：`Reclaimed` 落在 outbox 里，链路断开时它就在
    ///    那儿等着，重连后自然上报；否则断链期间的回收会让控制面永远看到一个
    ///    pending 的缺口。
    /// 3. **只回收连续前缀**：年龄未知（`at_unix == 0`）的条目保守不动，绝不从中间
    ///    挖洞——挖洞等于制造一个永远补不上的缺口。
    ///
    /// 返回被回收（水位线确有推进）的会话 id。
    pub fn sweep_retention(&mut self, retention_days: u32, now_unix: i64) -> Vec<String> {
        let cutoff = now_unix.saturating_sub(i64::from(retention_days) * 86_400);
        let mut swept = Vec::new();
        for (id, session) in self.sessions.iter_mut() {
            match session.log.reclaim_older_than(cutoff) {
                Ok(Some(watermark)) => {
                    self.outbox.push_back(SessionEvent::Reclaimed {
                        session_id: id.clone(),
                        reclaimed_through_seq: watermark,
                    });
                    swept.push(id.clone());
                }
                Ok(None) => {}
                Err(e) => eprintln!("sebas-node: 会话 {id} 保留期回收失败（跳过）：{e}"),
            }
        }
        swept
    }

    /// 推进某会话的回收水位线（4.4 的策略随后收紧；这里保证留下痕迹）。
    pub fn reclaim_through(&mut self, session_id: &str, through_seq: u64) -> Result<u64, Rejected> {
        let session = self.session_mut(session_id)?;
        session
            .log
            .reclaim_through(through_seq)
            .map_err(|e| Rejected::new(SessionRejectCode::NodeError, e.to_string()))
    }

    /// 取出到期的合并批（传输层）。
    pub fn drain_events(&mut self, now: Instant) -> Vec<SessionEvent> {
        // 先收执行体**自己**攒下的条目：真实 ACP 执行体在审批之后才继续产出，那些
        // 条目不会恰好落在本轮 `prompt` 的返回里（没有这一步，agent 的输出要等到
        // 下一次控制面操作才可能被看见）。
        let live: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.body.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in live {
            self.drain_body(&id);
        }

        // 审批 / 门控结论是**即时**事件（不受合并窗口约束）：先吐它们，
        // 否则一个等着决定的会话会被窗口无谓拖住。
        let mut immediate: Vec<SessionEvent> = self.outbox.drain(..).collect();
        let window = self.coalesce_window;
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| now.duration_since(p.first_at) >= window)
            .map(|(id, _)| id.clone())
            .collect();
        let mut events = Vec::new();
        for id in due {
            if let Some(pending) = self.pending.remove(&id) {
                if pending.entries.is_empty() {
                    continue;
                }
                let epoch = self
                    .sessions
                    .get(&id)
                    .map(|s| s.log.epoch())
                    .unwrap_or(0);
                events.push(SessionEvent::TurnBatch {
                    session_id: id,
                    epoch,
                    from_seq: pending.from_seq,
                    entries: pending.entries,
                    coalesced_overflow: pending.overflow,
                });
            }
        }
        events.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        immediate.extend(events);
        immediate
    }

    /// 处理一个链路请求，产出应答帧（add-remote-execution-node 3.5）。
    ///
    /// 这是「协议 → 宿主」的唯一入口：所有拒绝都翻成带码的 `Rejected`，因此
    /// 控制面拿到的永远是**可判别**的结果，而不是断链或静默无响应。
    pub fn handle(&mut self, id: u64, op: sebas_node_link::SessionOp) -> sebas_node_link::Frame {
        use sebas_node_link::{Frame, SessionOp, SessionResult};

        fn rejected(r: Rejected) -> SessionResult {
            SessionResult::Rejected {
                code: r.code,
                cause: r.cause,
            }
        }

        let result = match op {
            SessionOp::Spawn {
                session_id,
                project_dir,
                agent_kind,
                model,
                mode,
                provider,
            } => match self.spawn(
                &session_id,
                project_dir.as_deref(),
                agent_kind.as_deref(),
                model.as_deref(),
                mode.as_deref(),
                provider.as_deref(),
            ) {
                Ok(s) => SessionResult::Spawned {
                    epoch: s.epoch,
                    agent_kind: s.agent_kind,
                    model: s.model,
                    mode: s.mode,
                    provider: s.provider,
                    provider_cause: s.provider_cause,
                    materials_version: s.materials_version,
                },
                Err(r) => rejected(r),
            },
            SessionOp::Prompt { session_id, text } => match self.prompt(&session_id, &text) {
                Ok(()) => SessionResult::Ok,
                Err(r) => rejected(r),
            },
            SessionOp::Cancel { session_id } => match self.cancel(&session_id) {
                Ok(_) => SessionResult::Ok,
                Err(r) => rejected(r),
            },
            SessionOp::Close { session_id } => match self.close(&session_id) {
                Ok(()) => SessionResult::Ok,
                Err(r) => rejected(r),
            },
            SessionOp::SetModel { session_id, model_id } => {
                match self.set_model(&session_id, &model_id) {
                    Ok(model) => SessionResult::ModelSet { model },
                    Err(r) => rejected(r),
                }
            }
            SessionOp::LogFrom { session_id, from_seq } => {
                match self.log_from(&session_id, from_seq) {
                    Ok((epoch, entries, last_seq)) => SessionResult::Log {
                        epoch,
                        entries,
                        last_seq,
                    },
                    Err(r) => rejected(r),
                }
            }
            SessionOp::ListSessions => SessionResult::Sessions {
                sessions: self.list(),
            },
            SessionOp::Snapshot { session_id } => match self.snapshot(&session_id) {
                Ok((summary, epoch, last_seq, reclaimed_through_seq)) => SessionResult::Snapshot {
                    summary,
                    epoch,
                    last_seq,
                    reclaimed_through_seq,
                },
                Err(r) => rejected(r),
            },
            SessionOp::SetMode { session_id, mode } => {
                match self.set_mode(&session_id, &mode) {
                    // 复用 `ModelSet` 的形状而不新增应答变体：调用方按「回报实际生效
                    // 值」解读；强制不了时是 `None`（没有可声称生效的 mode）。
                    Ok(effective) => SessionResult::ModelSet {
                        model: effective.map(|m| m.as_str().to_string()),
                    },
                    Err(r) => rejected(r),
                }
            }
            SessionOp::ApprovalAnswer {
                session_id,
                request_id,
                decision,
            } => match self.answer_approval(&session_id, &request_id, decision) {
                Ok(applied) => SessionResult::ApprovalApplied { applied },
                Err(r) => rejected(r),
            },
            SessionOp::ParkedApprovals => SessionResult::ParkedApprovals {
                approvals: self.parked_approvals(),
            },
            SessionOp::FetchMaterials { .. } => SessionResult::Rejected {
                code: SessionRejectCode::NodeError,
                cause: "FetchMaterials 是**节点发起**的请求（向控制面拉材料），                        节点侧不处理它；方向搞反时如实拒绝而不是装作处理了"
                    .into(),
            },
            SessionOp::Ping => SessionResult::Pong,
            SessionOp::CheckPath { path } => match self.check_path(&path) {
                Ok((exists, is_dir)) => SessionResult::PathChecked { exists, is_dir },
                Err(r) => rejected(r),
            },
        };

        Frame::Response { id, result }
    }

    /// 「这个路径在**节点**上是什么？」（3.3 的同一条原则）。
    ///
    /// 判定规则刻意区分三种结果：
    /// - 有元数据 → `(exists, is_dir)`；
    /// - **确定不存在**（`NotFound`，或路径中间不是目录）→ `(false, false)`；
    /// - **读不到**（权限等）→ typed 拒绝，成因**指名路径**。
    ///
    /// 第三条是重点：把「读不到」报成「不存在」会让控制面以为路径没了，从而做错
    /// 决策（比如让操作者去建一个其实已存在的目录）。未知不等于不存在。
    pub fn check_path(&self, path: &str) -> Result<(bool, bool), Rejected> {
        if path.trim().is_empty() {
            return Err(Rejected::new(
                SessionRejectCode::UnusableProjectDir,
                "路径为空，无法判定",
            ));
        }
        match std::fs::metadata(path) {
            Ok(meta) => Ok((true, meta.is_dir())),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                Ok((false, false))
            }
            Err(e) => Err(Rejected::new(
                SessionRejectCode::UnusableProjectDir,
                format!("无法在节点上读取路径 {path} 的元数据（{e}）；不把它当作不存在"),
            )),
        }
    }

    /// 组装摘要。
    fn summary_of(&self, id: &str, session: &HostedSession) -> SessionSummary {
        SessionSummary {
            session_id: id.to_string(),
            phase: session.phase.clone(),
            agent_kind: session.body.as_ref().map(|b| b.agent_kind()),
            model: session.body.as_ref().and_then(|b| b.model()),
            // **实际生效**的模式；强制不了 → `None`（不把期望值回显成已生效）。
            mode: session.effective_mode.map(|m| m.as_str().to_string()),
            desired_mode: Some(session.desired_mode.as_str().to_string()),
            provider: session.provider.clone(),
            desired_provider: session.desired_provider.clone(),
            provider_cause: session.provider_cause.clone(),
            materials_version: session.materials_version.clone(),
            last_seq: session.log.last_seq(),
            epoch: session.log.epoch(),
        }
    }

    fn session_mut(&mut self, session_id: &str) -> Result<&mut HostedSession, Rejected> {
        self.sessions.get_mut(session_id).ok_or_else(|| {
            Rejected::new(
                SessionRejectCode::UnknownSession,
                format!("会话 {session_id} 不存在"),
            )
        })
    }

    /// 入队批次；超出单批上限即压缩并打标记（内容仍在日志里，可回拉）。
    fn push_pending(pending: &mut PendingBatch, recorded: &[LogEntry], max_entries: usize) {
        // 直接放**日志里的真条目**（含时间戳），不从事件重建影子条目。
        pending.entries.extend(recorded.iter().cloned());
        if pending.entries.len() > max_entries {
            let drop = pending.entries.len() - max_entries;
            pending.entries.drain(0..drop);
            pending.from_seq = pending.entries.first().map(|e| e.seq).unwrap_or(pending.from_seq);
            pending.overflow = true;
        }
    }

    /// 日志目录占用（存储上限判定）。
    fn disk_usage(&self) -> u64 {
        std::fs::read_dir(&self.log_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn host() -> (tempfile::TempDir, SessionHost) {
        let dir = tempfile::tempdir().unwrap();
        let host = SessionHost::new(dir.path(), 2, 0, std::sync::Arc::new(EchoOnlyFactory));
        (dir, host)
    }

    // ── CheckPath：路径可用性由**节点**判定（3.3 的同一条原则）────────────────
    ///
    /// 「读不到」与「不存在」是两回事：前者是 typed 拒绝，后者才是
    /// `exists:false`。把前者报成后者会让操作者以为路径没了。

    #[test]
    fn check_path_distinguishes_missing_file_and_directory() {
        let (d, host) = host();
        let dir = d.path().to_path_buf();
        assert_eq!(
            host.check_path(&dir.to_string_lossy()).unwrap(),
            (true, true),
            "目录"
        );

        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(
            host.check_path(&file.to_string_lossy()).unwrap(),
            (true, false),
            "存在但不是目录"
        );

        let missing = dir.join("nope");
        assert_eq!(
            host.check_path(&missing.to_string_lossy()).unwrap(),
            (false, false),
            "确定不存在"
        );

        // 路径中间不是目录（ENOTDIR）同样算「不存在」，不是「读不到」。
        let under_file = file.join("child");
        assert_eq!(host.check_path(&under_file.to_string_lossy()).unwrap(), (false, false));

        // 空路径什么都判定不了 → 如实拒绝。
        assert_eq!(
            host.check_path("   ").unwrap_err().code,
            SessionRejectCode::UnusableProjectDir
        );
    }

    #[test]
    fn check_path_goes_over_the_protocol_as_a_typed_answer() {
        use sebas_node_link::{Frame, SessionOp, SessionResult};
        let (d, mut host) = host();
        let dir = d.path().to_string_lossy().to_string();
        match host.handle(
            1,
            SessionOp::CheckPath {
                path: dir.clone(),
            },
        ) {
            Frame::Response {
                result: SessionResult::PathChecked { exists, is_dir },
                ..
            } => {
                assert!(exists && is_dir, "{dir} 应被判定为目录");
            }
            other => panic!("{other:?}"),
        }
        match host.handle(
            2,
            SessionOp::CheckPath {
                path: d.path().join("ghost").to_string_lossy().to_string(),
            },
        ) {
            Frame::Response {
                result: SessionResult::PathChecked { exists, is_dir },
                ..
            } => assert!(!exists && !is_dir),
            other => panic!("{other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_unstatable_path_is_a_typed_rejection_not_absent() {
        use std::os::unix::fs::PermissionsExt;
        let (d, host) = host();
        let locked = d.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let child = locked.join("child");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = host.check_path(&child.to_string_lossy());
        // 先恢复权限位，保证 tempdir 收得掉。
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));

        // root 能越权 stat，制造不出权限失败：如实说明并跳过，不编造结论。
        match &result {
            Err(_) => {}
            Ok(_) => {
                eprintln!(
                    "skipped: 当前用户可越权 stat（root？），无法制造「读不到」的情形"
                );
                return;
            }
        }
        let err = result.unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnusableProjectDir);
        assert!(
            err.cause.contains("child"),
            "成因必须指名路径：{}",
            err.cause
        );
    }

    // ── mode 的诚实边界与门控回执（6.3 / D6 / D7）──────────────────────────

    /// 停在门上的测试执行体：`prompt("gated")` 报一个带回执令牌的门。
    struct ParkBody {
        enforces: bool,
        resolved: Arc<Mutex<Vec<(String, ApprovalDecision)>>>,
        closed: bool,
    }

    struct ParkFactory {
        enforces: bool,
        resolved: Arc<Mutex<Vec<(String, ApprovalDecision)>>>,
    }

    impl BodyFactory for ParkFactory {
        fn make(&self, _spec: &BodySpec<'_>) -> Result<MadeBody, Rejected> {
            Ok(MadeBody::plain(Box::new(ParkBody {
                enforces: self.enforces,
                resolved: Arc::clone(&self.resolved),
                closed: false,
            })))
        }
        fn available_kinds(&self) -> Vec<String> {
            vec!["park".into()]
        }
    }

    impl ExecutionBody for ParkBody {
        fn agent_kind(&self) -> String {
            "park".into()
        }
        fn model(&self) -> Option<String> {
            None
        }
        fn mode(&self) -> Option<String> {
            None
        }
        fn enforces_mode(&self) -> bool {
            self.enforces
        }
        fn prompt(&mut self, text: &str, out: &mut Vec<BodyEvent>) -> Result<(), String> {
            out.push(BodyEvent {
                kind: "output".into(),
                text: format!("park: {text}"),
                data: None,
                gate: None,
            });
            if text == "gated" {
                out.push(BodyEvent {
                    kind: "permission_request".into(),
                    text: "bash".into(),
                    data: None,
                    gate: Some(GateRequest {
                        tool: "bash".into(),
                        category: GateCategory::Execute,
                        resume_token: Some("body-token".into()),
                    }),
                });
            }
            Ok(())
        }
        fn cancel(&mut self) -> Result<bool, String> {
            Ok(false)
        }
        fn set_model(&mut self, _model: &str) -> Result<Option<String>, String> {
            Err("测试执行体不支持切换模型".into())
        }
        fn close(&mut self) {
            self.closed = true;
        }
        fn resolve_gate(
            &mut self,
            resume_token: &str,
            decision: ApprovalDecision,
        ) -> Result<(), String> {
            self.resolved
                .lock()
                .unwrap()
                .push((resume_token.to_string(), decision));
            Ok(())
        }
    }

    fn park_host(enforces: bool) -> (tempfile::TempDir, SessionHost, Arc<Mutex<Vec<(String, ApprovalDecision)>>>) {
        let dir = tempfile::tempdir().unwrap();
        let resolved = Arc::new(Mutex::new(Vec::new()));
        let factory = Arc::new(ParkFactory {
            enforces,
            resolved: Arc::clone(&resolved),
        });
        let host = SessionHost::new(dir.path(), 4, 0, factory);
        (dir, host, resolved)
    }

    #[test]
    fn a_control_plane_decision_is_delivered_back_to_the_parked_body() {
        let (_d, mut host, resolved) = park_host(true);
        host.spawn("s-1", None, Some("park"), None, Some("ask"), None)
            .unwrap();
        host.prompt("s-1", "gated").unwrap();
        assert_eq!(host.parked_approvals().len(), 1);
        let request_id = host.parked_approvals()[0].request_id.clone();

        assert!(host
            .answer_approval("s-1", &request_id, ApprovalDecision::Deny)
            .unwrap());
        assert_eq!(
            *resolved.lock().unwrap(),
            vec![("body-token".to_string(), ApprovalDecision::Deny)],
            "决定必须送回停在门上的执行体（否则 agent 永远等下去）"
        );
    }

    #[test]
    fn an_auto_allowed_gate_is_also_delivered_to_the_body() {
        let (_d, mut host, resolved) = park_host(true);
        host.spawn("s-1", None, Some("park"), None, Some("allow"), None)
            .unwrap();
        host.prompt("s-1", "gated").unwrap();

        assert!(host.parked_approvals().is_empty(), "allow 不放门上停");
        assert_eq!(
            *resolved.lock().unwrap(),
            vec![("body-token".to_string(), ApprovalDecision::AllowOnce)],
            "模式自动放行也要回执，否则执行体一直卡在门上"
        );
    }

    #[test]
    fn a_body_that_cannot_enforce_mode_reports_no_effective_mode_but_still_gates() {
        let (_d, mut host, _resolved) = park_host(false);
        let spawned = host
            .spawn("s-1", None, Some("park"), None, Some("ask"), None)
            .unwrap();
        assert_eq!(
            spawned.mode, None,
            "强制不了就如实回「没有可声称生效的 mode」，不回显期望值"
        );

        let (summary, ..) = host.snapshot("s-1").unwrap();
        assert_eq!(summary.mode, None);
        assert_eq!(summary.desired_mode.as_deref(), Some("ask"), "期望值仍可见");

        // 仍按期望模式**尽力**门控看得见的请求：ask 之下请求照旧停驻。
        host.prompt("s-1", "gated").unwrap();
        assert_eq!(host.parked_approvals().len(), 1);

        // set_mode 同样如实回报「实际生效」。
        assert_eq!(host.set_mode("s-1", "auto").unwrap(), None);
    }

    #[test]
    fn a_dying_body_terminates_the_session_but_keeps_the_log() {
        struct DeadBody;
        impl ExecutionBody for DeadBody {
            fn agent_kind(&self) -> String {
                "dying".into()
            }
            fn model(&self) -> Option<String> {
                None
            }
            fn mode(&self) -> Option<String> {
                None
            }
            fn prompt(&mut self, _text: &str, out: &mut Vec<BodyEvent>) -> Result<(), String> {
                out.push(BodyEvent {
                    kind: "error".into(),
                    text: "子进程没了".into(),
                    data: Some(serde_json::json!({ "terminal": true })),
                    gate: None,
                });
                Ok(())
            }
            fn cancel(&mut self) -> Result<bool, String> {
                Ok(false)
            }
            fn set_model(&mut self, _model: &str) -> Result<Option<String>, String> {
                Err("不支持".into())
            }
            fn close(&mut self) {}
        }
        struct DeadFactory;
        impl BodyFactory for DeadFactory {
            fn make(&self, _spec: &BodySpec<'_>) -> Result<MadeBody, Rejected> {
                Ok(MadeBody::plain(Box::new(DeadBody)))
            }
            fn available_kinds(&self) -> Vec<String> {
                vec!["dying".into()]
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut host = SessionHost::new(dir.path(), 4, 0, Arc::new(DeadFactory));
        host.spawn("s-1", None, Some("dying"), None, None, None)
            .unwrap();
        host.prompt("s-1", "x").unwrap();

        let (summary, ..) = host.snapshot("s-1").unwrap();
        assert_eq!(summary.phase, "exited", "子进程死亡即会话终结");
        // 执行事实不因死亡消失：日志仍可拉。
        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        assert!(entries.iter().any(|e| e.text.contains("子进程没了")));
        // 终结后的输入是明确的 SessionClosed（不是 UnknownSession）。
        assert_eq!(
            host.prompt("s-1", "again").unwrap_err().code,
            SessionRejectCode::SessionClosed
        );
    }

    #[test]
    fn spawn_records_state_and_reports_effective_values() {
        let (_d, mut host) = host();
        let spawned = host
            .spawn("s-1", None, Some("echo"), Some("m1"), Some("ask"), None)
            .unwrap();
        assert_eq!(spawned.agent_kind, "echo");
        assert_eq!(spawned.model.as_deref(), Some("m1"));
        assert_eq!(spawned.mode.as_deref(), Some("ask"));
        assert_eq!(spawned.epoch, 1);

        // 日志里有 spawning → active 两条状态。
        let (epoch, entries, last) = host.log_from("s-1", 1).unwrap();
        assert_eq!(epoch, 1);
        assert_eq!(last, 2);
        assert_eq!(entries[0].text, "spawning");
        assert_eq!(entries[1].text, "active");
    }

    #[test]
    fn prompt_writes_prompt_and_output_in_order() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        host.prompt("s-1", "hello").unwrap();
        let (_, entries, last) = host.log_from("s-1", 3).unwrap();
        assert_eq!(last, 4);
        assert_eq!(entries[0].kind, "prompt");
        assert_eq!(entries[0].text, "hello");
        assert_eq!(entries[1].kind, "output");
        assert_eq!(entries[1].text, "echo: hello");
    }

    #[test]
    fn duplicate_session_is_refused() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        let err = host
            .spawn("s-1", None, Some("echo"), None, None, None)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::DuplicateSession);
        assert!(err.code.is_permanent());
    }

    #[test]
    fn capacity_is_enforced_and_reported_as_transient() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        host.spawn("s-2", None, Some("echo"), None, None, None).unwrap();
        let err = host
            .spawn("s-3", None, Some("echo"), None, None, None)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::OverCapacity);
        assert!(!err.code.is_permanent(), "容量是瞬时的：等会话结束即可");
        // 关掉一个就能再开。
        host.close("s-1").unwrap();
        assert!(host.spawn("s-3", None, Some("echo"), None, None, None).is_ok());
    }

    #[test]
    fn project_dir_is_validated_on_the_node() {
        let (d, mut host) = host();
        let missing = d.path().join("nope");
        let err = host
            .spawn(
                "s-1",
                Some(&missing.to_string_lossy()),
                Some("echo"),
                None,
                None,
                None,
            )
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnusableProjectDir);
        assert!(err.cause.contains("nope"), "{}", err.cause);

        // 存在的目录可用。
        assert!(
            host.spawn(
                "s-2",
                Some(&d.path().to_string_lossy()),
                Some("echo"),
                None,
                None,
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn unknown_agent_kind_is_refused_honestly() {
        let (_d, mut host) = host();
        let err = host
            .spawn("s-1", None, Some("claude"), None, None, None)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnsupportedAgentKind);
        assert!(err.cause.contains("claude"), "{}", err.cause);
        assert_eq!(host.available_kinds(), vec!["echo".to_string()]);
    }

    #[test]
    fn unknown_session_operations_are_refused() {
        let (_d, mut host) = host();
        for err in [
            host.prompt("nope", "x").unwrap_err(),
            host.cancel("nope").unwrap_err(),
            host.close("nope").unwrap_err(),
            host.set_model("nope", "m").unwrap_err(),
            host.log_from("nope", 1).unwrap_err(),
            host.snapshot("nope").unwrap_err(),
        ] {
            assert_eq!(err.code, SessionRejectCode::UnknownSession);
        }
    }

    #[test]
    fn set_model_reports_the_effective_value() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        assert_eq!(
            host.set_model("s-1", "m2").unwrap().as_deref(),
            Some("m2")
        );
        let (summary, ..) = host.snapshot("s-1").unwrap();
        assert_eq!(summary.model.as_deref(), Some("m2"));
    }

    #[test]
    fn batches_are_coalesced_within_the_window_and_exact_on_pull() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = SessionHost::new(dir.path(), 4, 0, std::sync::Arc::new(EchoOnlyFactory))
            .with_coalescing(Duration::from_millis(50), 256);
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();

        for i in 0..5 {
            host.prompt("s-1", &format!("p{i}")).unwrap();
        }
        // 窗口未到：不产出批次。
        assert!(host.drain_events(Instant::now()).is_empty());
        // 窗口到：一个批次包含全部条目。
        let events = host.drain_events(Instant::now() + Duration::from_millis(60));
        assert_eq!(events.len(), 1);
        match &events[0] {
            SessionEvent::TurnBatch {
                session_id,
                entries,
                coalesced_overflow,
                ..
            } => {
                assert_eq!(session_id, "s-1");
                assert_eq!(entries.len(), 10, "5 轮 × (prompt+output)");
                assert!(!coalesced_overflow);
            }
            other => panic!("{other:?}"),
        }
        // 合并只影响传输：回拉仍拿到精确序列。
        let (_, exact, last) = host.log_from("s-1", 1).unwrap();
        assert_eq!(exact.len() as u64, last);
        assert_eq!(exact[2].text, "p0");
    }

    #[test]
    fn batch_overflow_is_marked_not_silently_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = SessionHost::new(dir.path(), 4, 0, std::sync::Arc::new(EchoOnlyFactory))
            .with_coalescing(Duration::from_millis(1), 4);
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        for i in 0..5 {
            host.prompt("s-1", &format!("p{i}")).unwrap();
        }
        let events = host.drain_events(Instant::now() + Duration::from_millis(5));
        match &events[0] {
            SessionEvent::TurnBatch {
                entries,
                coalesced_overflow,
                from_seq,
                ..
            } => {
                assert!(coalesced_overflow, "超限必须打标记");
                assert_eq!(entries.len(), 4, "批内条目被压到上限");
                assert!(*from_seq > 1, "批次起点应前移到保留段");
            }
            other => panic!("{other:?}"),
        }
        // 被压掉的条目仍在日志里（可回拉），没有被丢掉。
        let (_, exact, _) = host.log_from("s-1", 1).unwrap();
        assert!(exact.len() >= 10, "原始序列完整：{}", exact.len());
    }

    #[test]
    fn reclaim_advances_the_watermark_and_reset_bumps_epoch() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        host.prompt("s-1", "x").unwrap();
        let (_, _, last, reclaimed) = host.snapshot("s-1").unwrap();
        assert_eq!(reclaimed, 0);
        assert_eq!(host.reclaim_through("s-1", last).unwrap(), last);
        let (_, _, _, reclaimed) = host.snapshot("s-1").unwrap();
        assert_eq!(reclaimed, last);
    }

    #[test]
    fn retention_sweep_reclaims_aged_prefix_and_reports_the_watermark() {
        let dir = tempfile::tempdir().unwrap();
        // 用真实"现在"：保留期的判据是「距今多少天」，编一个遥远的时间点只会
        // 把当下刚落的账也判成过期，那是测试构造的假象，不是被测行为。
        let now = crate::log::now_unix();
        let day = 86_400i64;

        // 造一段**真实形状**的历史：一个 40 天前的会话，日志躺在磁盘上而内存里
        // 没有——正是节点重启后挂回来的样子（保留期回收要处理的通常就是它）。
        let aged_through = {
            let mut log = SessionLog::open(dir.path(), "s-old").unwrap();
            let a = log
                .append_at("output", "aged-1", None, now - 40 * day)
                .unwrap()
                .seq;
            let b = log
                .append_at("output", "aged-2", None, now - 39 * day)
                .unwrap()
                .seq;
            assert_eq!(b, a + 1, "追加的 seq 连续");
            b
        };

        let mut host = SessionHost::new(dir.path(), 4, 0, std::sync::Arc::new(EchoOnlyFactory));
        assert_eq!(
            host.list().len(),
            1,
            "老会话应已被挂回（否则它的历史拉不到）"
        );
        // 当下的会话：条目新鲜，不该被回收。
        host.spawn("s-new", None, Some("echo"), None, None, None).unwrap();

        let swept = host.sweep_retention(30, now);
        assert_eq!(swept, vec!["s-old".to_string()], "只回收真有推进的会话");

        // 回收只吃掉老前缀，seq 连续性不受影响（控制面据 last_seq 判连续性）。
        let (_, entries, last) = host.log_from("s-old", 1).unwrap();
        assert!(entries.is_empty(), "40 天前的条目都该被回收：{entries:?}");
        assert_eq!(last, aged_through, "回收不影响 seq 连续性");
        let (_, _, _, reclaimed) = host.snapshot("s-old").unwrap();
        assert_eq!(reclaimed, aged_through, "水位线进了快照，快照路径也看得到");
        assert_eq!(
            host.snapshot("s-new").unwrap().3,
            0,
            "新鲜会话的水位线不该动"
        );

        // 水位线必须上报：控制面据此把该段标为「节点已回收」而不是 pending。
        let events = host.drain_events(Instant::now());
        let reported: Vec<(String, u64)> = events
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::Reclaimed {
                    session_id,
                    reclaimed_through_seq,
                } => Some((session_id, reclaimed_through_seq)),
                _ => None,
            })
            .collect();
        assert_eq!(
            reported,
            vec![("s-old".to_string(), aged_through)],
            "回收必须上报水位线"
        );

        // 幂等：没有可回收段时什么都不做，也不产生噪音事件。
        assert!(host.sweep_retention(30, now).is_empty());
        assert!(
            !host
                .drain_events(Instant::now())
                .iter()
                .any(|e| matches!(e, SessionEvent::Reclaimed { .. })),
            "无事可做时不该重复上报"
        );
    }

    #[test]
    fn storage_ceiling_refuses_new_sessions_without_dropping_history() {
        let dir = tempfile::tempdir().unwrap();
        // 上限设为 1 字节：任何已落盘的日志都会让它触顶。
        let mut host = SessionHost::new(dir.path(), 4, 1, std::sync::Arc::new(EchoOnlyFactory));
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        let err = host
            .spawn("s-2", None, Some("echo"), None, None, None)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::StorageExhausted);
        assert!(!err.code.is_permanent(), "存储是可恢复的（清理/扩容）");
        // 既有会话的历史没被动过。
        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        assert!(!entries.is_empty(), "触顶只拒绝新工作，不丢历史");
    }

    #[test]
    fn list_and_snapshot_agree() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        host.spawn("s-2", None, Some("echo"), None, None, None).unwrap();
        let listed = host.list();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].session_id, "s-1");
        let (summary, epoch, last, _) = host.snapshot("s-1").unwrap();
        assert_eq!(summary.session_id, "s-1");
        assert_eq!(summary.epoch, epoch);
        assert_eq!(summary.last_seq, last);
    }

    #[test]
    fn close_terminates_the_body_but_keeps_the_log_readable() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        host.prompt("s-1", "before-close").unwrap();
        host.close("s-1").unwrap();

        // 执行事实不因关闭消失：日志仍可拉、快照仍可查。
        let (_, entries, last) = host.log_from("s-1", 1).unwrap();
        assert!(entries.iter().any(|e| e.text == "before-close"));
        assert_eq!(entries.last().unwrap().text, "closed");
        let (summary, ..) = host.snapshot("s-1").unwrap();
        assert_eq!(summary.phase, "closed");
        assert_eq!(summary.last_seq, last);

        // 但不再接受输入：明确的 SessionClosed（不是 UnknownSession）。
        assert_eq!(
            host.prompt("s-1", "x").unwrap_err().code,
            SessionRejectCode::SessionClosed
        );
        assert_eq!(
            host.set_model("s-1", "m").unwrap_err().code,
            SessionRejectCode::SessionClosed
        );
        // 重复 close 幂等。
        host.close("s-1").unwrap();
    }

    #[test]
    fn closed_sessions_do_not_consume_capacity() {
        let (_d, mut host) = host(); // 上限 2
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        host.spawn("s-2", None, Some("echo"), None, None, None).unwrap();
        assert_eq!(
            host.spawn("s-3", None, Some("echo"), None, None, None)
                .unwrap_err()
                .code,
            SessionRejectCode::OverCapacity
        );
        host.close("s-1").unwrap();
        assert!(
            host.spawn("s-3", None, Some("echo"), None, None, None).is_ok(),
            "关闭释放并发额度，但 s-1 的日志仍在"
        );
        assert!(host.log_from("s-1", 1).is_ok());
    }

    #[test]
    fn handle_routes_ops_and_returns_typed_rejections() {
        use sebas_node_link::{Frame, SessionOp, SessionResult};
        let (_d, mut host) = host();

        // spawn → Spawned
        match host.handle(
            1,
            SessionOp::Spawn {
                session_id: "s-1".into(),
                project_dir: None,
                agent_kind: Some("echo".into()),
                model: None,
                mode: None,
                provider: None,
            },
        ) {
            Frame::Response {
                id: 1,
                result: SessionResult::Spawned { agent_kind, .. },
            } => assert_eq!(agent_kind, "echo"),
            other => panic!("{other:?}"),
        }

        // prompt → Ok，且日志里真的有内容
        assert!(matches!(
            host.handle(
                2,
                SessionOp::Prompt {
                    session_id: "s-1".into(),
                    text: "hi".into()
                }
            ),
            Frame::Response {
                result: SessionResult::Ok,
                ..
            }
        ));

        // log_from → Log（精确序列）
        match host.handle(
            3,
            SessionOp::LogFrom {
                session_id: "s-1".into(),
                from_seq: 1,
            },
        ) {
            Frame::Response {
                result: SessionResult::Log { entries, last_seq, .. },
                ..
            } => {
                assert_eq!(entries.len() as u64, last_seq);
                assert!(entries.iter().any(|e| e.text == "echo: hi"));
            }
            other => panic!("{other:?}"),
        }

        // set_model → ModelSet（实际生效值）
        match host.handle(
            4,
            SessionOp::SetModel {
                session_id: "s-1".into(),
                model_id: "m9".into(),
            },
        ) {
            Frame::Response {
                result: SessionResult::ModelSet { model },
                ..
            } => assert_eq!(model.as_deref(), Some("m9")),
            other => panic!("{other:?}"),
        }

        // list / snapshot / ping
        assert!(matches!(
            host.handle(5, SessionOp::ListSessions),
            Frame::Response {
                result: SessionResult::Sessions { .. },
                ..
            }
        ));
        assert!(matches!(
            host.handle(
                6,
                SessionOp::Snapshot {
                    session_id: "s-1".into()
                }
            ),
            Frame::Response {
                result: SessionResult::Snapshot { .. },
                ..
            }
        ));
        assert!(matches!(
            host.handle(7, SessionOp::Ping),
            Frame::Response {
                result: SessionResult::Pong,
                ..
            }
        ));

        // 未知会话：可判别地拒绝，而不是断链或静默。
        match host.handle(
            8,
            SessionOp::Prompt {
                session_id: "ghost".into(),
                text: "x".into(),
            },
        ) {
            Frame::Response {
                result: SessionResult::Rejected { code, cause },
                ..
            } => {
                assert_eq!(code, SessionRejectCode::UnknownSession);
                assert!(cause.contains("ghost"), "{cause}");
            }
            other => panic!("{other:?}"),
        }
    }

    // ── mode 门与审批（group 6）────────────────────────────────────────────

    /// 取某会话新的即时事件（审批/门控结论）。
    fn drain_outbox(host: &mut SessionHost) -> Vec<SessionEvent> {
        host.drain_events(Instant::now() + Duration::from_secs(1))
    }

    #[test]
    fn the_default_mode_is_ask_not_auto() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, None, None).unwrap();
        let (summary, ..) = host.snapshot("s-1").unwrap();
        assert_eq!(summary.mode.as_deref(), Some("ask"), "auto 永远不是缺省");
    }

    #[test]
    fn ask_mode_parks_a_gated_action_and_reports_it() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        host.prompt("s-1", "run: ls -la").unwrap();

        // 停驻：请求在队列里，工具没跑。
        let parked = host.parked_approvals();
        assert_eq!(parked.len(), 1, "ask 模式下受门控动作应停驻");
        assert_eq!(parked[0].tool, "bash");
        assert_eq!(parked[0].category, GateCategory::Execute);
        assert_eq!(parked[0].mode, SessionMode::Ask);
        assert_eq!(parked[0].session_id, "s-1");

        // 上报事件确实发出去了（上行走廊）。
        let events = drain_outbox(&mut host);
        assert!(
            events.iter().any(|e| matches!(
                e,
                SessionEvent::ApprovalRequested { request_id, tool, .. }
                    if tool == "bash" && request_id.ends_with(":req-1")
            )),
            "{events:?}"
        );

        // 工具没执行：日志里没有 output，只有请求记录。
        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        assert!(entries.iter().any(|e| e.kind == "approval_requested"));
        assert!(
            !entries.iter().any(|e| e.kind == "output"),
            "未获批准的动作不得产生执行痕迹：{entries:?}"
        );
    }

    #[test]
    fn auto_mode_allows_without_asking_but_leaves_an_audit_trail() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("auto"), None).unwrap();
        host.prompt("s-1", "run: rm -rf /tmp/x").unwrap();

        assert!(host.parked_approvals().is_empty(), "auto 之下不产生审批请求");
        let events = drain_outbox(&mut host);
        assert!(
            events.iter().any(|e| matches!(
                e,
                SessionEvent::GateResolved { decision, source, .. }
                    if decision == "auto_allowed" && source == "mode:auto"
            )),
            "自动放行必须说明是谁放行的：{events:?}"
        );

        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        let audit = entries
            .iter()
            .find(|e| e.kind == "audit")
            .expect("自动放行必须留审计");
        assert!(audit.text.contains("auto"), "{}", audit.text);
        assert!(audit.text.contains("bash"), "{}", audit.text);
    }

    #[test]
    fn allow_mode_is_ungated_for_execution_but_edit_only_for_edits() {
        // allow：执行类也不问。
        let (_d, mut allow_host) = host();
        allow_host
            .spawn("s-1", None, Some("echo"), None, Some("allow"), None)
            .unwrap();
        allow_host.prompt("s-1", "run: ls").unwrap();
        assert!(
            allow_host.parked_approvals().is_empty(),
            "allow 对执行类放行"
        );
        drain_outbox(&mut allow_host);

        // edit：执行类仍然要问（只有编辑类放行）。
        let (_d2, mut edit_host) = host();
        edit_host
            .spawn("s-1", None, Some("echo"), None, Some("edit"), None)
            .unwrap();
        edit_host.prompt("s-1", "run: ls").unwrap();
        assert_eq!(edit_host.parked_approvals().len(), 1, "edit 不覆盖执行类");
    }

    #[test]
    fn an_approval_answer_resolves_the_parked_request_once() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        host.prompt("s-1", "run: ls").unwrap();
        drain_outbox(&mut host);
        let request_id = host.parked_approvals()[0].request_id.clone();

        assert!(
            host.answer_approval("s-1", &request_id, ApprovalDecision::AllowOnce)
                .unwrap(),
            "决定应生效"
        );
        assert!(host.parked_approvals().is_empty(), "决议后不再悬空");

        // 审计：谁给的结论、什么结论。
        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        let audit = entries
            .iter()
            .filter(|e| e.kind == "audit")
            .find(|e| e.text.contains("allow_once"))
            .expect("决议必须留审计");
        assert!(audit.text.contains(&request_id), "{}", audit.text);

        // 结论也上报了，来源标明是控制面。
        let events = drain_outbox(&mut host);
        assert!(
            events.iter().any(|e| matches!(
                e,
                SessionEvent::GateResolved { decision, source, .. }
                    if decision == "allow_once" && source == "control-plane"
            )),
            "{events:?}"
        );

        // 同一个 id 再来一次：可判别拒绝（不重复生效）。
        let err = host
            .answer_approval("s-1", &request_id, ApprovalDecision::AllowOnce)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnknownApprovalRequest);
    }

    #[test]
    fn an_unknown_decision_id_is_refused() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        let err = host
            .answer_approval("s-1", "s-1:req-999", ApprovalDecision::Deny)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnknownApprovalRequest);
        assert!(err.code.is_permanent());
    }

    #[test]
    fn a_decision_for_a_closed_session_is_discarded_with_a_notice() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        host.prompt("s-1", "run: ls").unwrap();
        drain_outbox(&mut host);
        let request_id = host.parked_approvals()[0].request_id.clone();
        host.close("s-1").unwrap();

        assert!(
            !host
                .answer_approval("s-1", &request_id, ApprovalDecision::AllowOnce)
                .unwrap(),
            "已关闭会话的决定不得生效"
        );
        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.kind == "audit" && e.text.contains("丢弃迟到决定")),
            "丢弃必须留痕：{entries:?}"
        );
    }

    #[test]
    fn there_is_no_local_decision_path_when_the_control_plane_is_away() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        host.prompt("s-1", "run: ls").unwrap();

        // 把合并窗口推到一小时后：仍然**没有任何**结论产生——不存在超时拒绝、
        // 也不存在超时放行（主控不可达时无限期 park）。
        let events = host.drain_events(Instant::now() + Duration::from_secs(3600));
        assert!(
            !events.iter().any(|e| matches!(e, SessionEvent::GateResolved { .. })),
            "不得自行给出门控结论：{events:?}"
        );
        assert_eq!(host.parked_approvals().len(), 1, "请求仍停驻");

        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        assert!(
            !entries
                .iter()
                .any(|e| e.kind == "audit" && (e.text.contains("allow") || e.text.contains("deny"))),
            "没有裁决路径就不该有裁决痕迹：{entries:?}"
        );
    }

    #[test]
    fn unknown_modes_are_refused_instead_of_silently_downgraded() {
        let (_d, mut host) = host();
        let err = host
            .spawn("s-1", None, Some("echo"), None, Some("yolo"), None)
            .unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnsupportedMode);
        assert!(err.cause.contains("yolo"), "{}", err.cause);

        host.spawn("s-2", None, Some("echo"), None, None, None).unwrap();
        let err = host.set_mode("s-2", "yolo").unwrap_err();
        assert_eq!(err.code, SessionRejectCode::UnsupportedMode);
        let (summary, ..) = host.snapshot("s-2").unwrap();
        assert_eq!(summary.mode.as_deref(), Some("ask"), "拒绝后模式不变");
    }

    #[test]
    fn turning_on_auto_is_audited() {
        let (_d, mut host) = host();
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        let effective = host.set_mode("s-1", "auto").unwrap();
        assert_eq!(effective, Some(SessionMode::Auto));

        let (_, entries, _) = host.log_from("s-1", 1).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.kind == "audit" && e.text.contains("mode=auto")),
            "开启 auto 必须留审计：{entries:?}"
        );

        // 之后受门控动作不再产生请求。
        host.prompt("s-1", "run: ls").unwrap();
        assert!(host.parked_approvals().is_empty());
    }

    #[test]
    fn parked_approvals_span_sessions_for_reconcile() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = SessionHost::new(dir.path(), 4, 0, std::sync::Arc::new(EchoOnlyFactory));
        host.spawn("s-1", None, Some("echo"), None, Some("ask"), None).unwrap();
        host.spawn("s-2", None, Some("echo"), None, Some("ask"), None).unwrap();
        host.prompt("s-1", "run: one").unwrap();
        host.prompt("s-2", "run: two").unwrap();

        let parked = host.parked_approvals();
        assert_eq!(parked.len(), 2, "对账要能看到所有会话的悬空请求");
        assert_eq!(parked[0].session_id, "s-1");
        assert_eq!(parked[1].session_id, "s-2");
    }
}
