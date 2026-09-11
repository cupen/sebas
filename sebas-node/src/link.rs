//! 节点侧链路：**出站**拨号、握手、退避重连、配对换取长期凭据。
//!
//! 反向连接的理由（设计 D13）：节点常在内网/别人的机器上，主控不该需要能反向访问
//! 它；因此节点主动拨出，主控只暴露一个端点。
//!
//! 退避口径沿用 `feishu-bridge` 的长连接约定：1 秒起、逐次翻倍、封顶 60 秒、成功即
//! 复位。**永久拒绝不重试**——判定依据来自协议本身（`RejectCode::is_permanent`），
//! 不让两端各自猜。
//!
//! `wss://` 尚未实现（需要 TLS 栈）。设计里 TLS 由部署方终止（反代/VPN），因此可行
//! 形态是节点连本地反代的 `ws://`；对 `wss://` 我们**如实拒绝**而不是假装支持。

use crate::body::{RouterCell, RouterEndpoint};
use crate::error::NodeError;
use crate::identity::{IdentityStore, NodeId};
use futures_util::{SinkExt, StreamExt};
use crate::materials::MaterialStore;
use crate::session::{BodyFactory, EchoOnlyFactory, SessionHost};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot};
use sebas_node_link::{
    CapabilityManifest, Frame, Hello, HelloAck, HelloOutcome, NodeAuth, SessionOp, SessionResult,
    PROTOCOL_VERSION,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// 默认退避起点（与 feishu-bridge 一致）。
pub const DEFAULT_BACKOFF_START: Duration = Duration::from_secs(1);
/// 默认退避上限（与 feishu-bridge 一致）。
pub const DEFAULT_BACKOFF_CAP: Duration = Duration::from_secs(60);
/// 握手应答超时：超过即按瞬时失败退避重试（不无限吊死）。
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// 材料拉取超时（会话创建在被门内，不能无限等）。
pub const MATERIALS_TIMEOUT: Duration = Duration::from_secs(10);

/// 保留期回收的巡检周期（4.4）。
///
/// 保留期以「天」计，小时级巡检足够；`interval` 的首次 tick 立即触发，因此启动时
/// 也会扫一遍（把停机期间过期的前缀一并回收）。
pub const RETENTION_SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// 可注入的链路调参（测试用短退避，生产用默认值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkTuning {
    /// 退避起点。
    pub backoff_start: Duration,
    /// 退避上限。
    pub backoff_cap: Duration,
    /// 握手应答超时。
    pub handshake_timeout: Duration,
    /// 保留期回收巡检周期。
    pub retention_sweep: Duration,
}

impl Default for LinkTuning {
    fn default() -> Self {
        Self {
            backoff_start: DEFAULT_BACKOFF_START,
            backoff_cap: DEFAULT_BACKOFF_CAP,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            retention_sweep: RETENTION_SWEEP_INTERVAL,
        }
    }
}

/// 下一次退避：翻倍但封顶。
pub fn next_backoff(current: Duration, cap: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > cap { cap } else { doubled }
}

/// 「巡检任务绝不能活得比链路任务久」的守卫。
///
/// 靠的是 drop：外层任务被 `abort` 时局部变量会照常析构，于是 drop 里 abort 巡检。
/// 不这样做会漏掉一条**真链路**：巡检持有 `Arc<LinkClient>`，而 `outbound` 里
/// 存着写任务的 sender——`LinkClient` 不析构，sender 就一直在，写任务
/// `out_rx.recv()` 永不返回，套接字**永不关闭**。对端（主控）于是看到一个
/// 「任务已经没了、连接却还连着」的节点：既不回话，也不下线（e2e 抓到的正是
/// 「节点未在预期时间内下线」）。
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// 一次拨号尝试的结果。
pub enum Attempt {
    /// 链路已建立。会话协议（prompt / turn 流 / 对账）属于后续 group，此处只持有连接。
    Connected(WebSocketStream<MaybeTlsStream<TcpStream>>),
    /// 永久失败：不该重试。
    Fatal(NodeError),
    /// 瞬时失败：应退避重试。
    Transient(String),
}

/// 节点侧链路客户端。
pub struct LinkClient {
    control_plane: String,
    node_id: NodeId,
    store: IdentityStore,
    /// 一次性配对 token。**配对成功即作废**（见 `auth`）：token 在主控侧只有一次
    /// 生命，重连时再拿它去握手会被回 `join_token_consumed`——那是个*永久*拒绝，
    /// 于是节点会在第一次断链后彻底退出。互斥是为了让"用掉就作废"这件事在
    /// `attempt(&self)` 里也能发生。
    join_token: std::sync::Mutex<Option<String>>,
    manifest: CapabilityManifest,
    tuning: LinkTuning,
    /// 本地日志保留天数（4.4 / r2）。**节点自主**的保留策略，由节点配置决定。
    retention_days: u32,
    /// 会话宿主。**跨连接存活**——链路断开不终止会话（rung ③⁺），因此它属于
    /// 客户端（进程级）而不是某一次连接。
    host: tokio::sync::Mutex<SessionHost>,
    /// 操作者级材料的落地仓（7.3）。
    materials: MaterialStore,
    /// 当前连接的出站通道（断开时清空）。节点用它向控制面发请求（材料拉取）。
    outbound: tokio::sync::Mutex<Option<mpsc::UnboundedSender<Message>>>,
    /// 在飞的**节点→控制面**请求（按 id 关联）。
    pending: tokio::sync::Mutex<HashMap<u64, oneshot::Sender<SessionResult>>>,
    next_request_id: AtomicU64,
    /// Spawn 路径的串行锁。
    ///
    /// 请求现在由**独立任务**处理（否则拉材料会堵住读循环——那是个死锁：等应答的同时
    /// 堵着接收应答的那条循环）。但「拉材料 → 交给宿主钉版本 → 建会话」这一串必须
    /// 原子，否则两个并发 Spawn 会让材料槽串味。
    spawn_serial: tokio::sync::Mutex<()>,
    /// 控制面在握手里告知的 router 端点（7.2）：握手成功填入、断链清空。
    ///
    /// 与执行体工厂共享同一份槽（`NodeBodyFactory::router_cell`），因此会话创建时
    /// 拿到的是**当前连接**的地址，而不是某个历史地址。
    router: RouterCell,
}

impl LinkClient {
    /// 用主控端点、身份与凭据存储构造。
    pub fn new(
        control_plane: impl Into<String>,
        node_id: NodeId,
        store: IdentityStore,
        join_token: Option<String>,
        sessions_dir: PathBuf,
        materials_dir: PathBuf,
        max_sessions: usize,
    ) -> Self {
        Self {
            control_plane: control_plane.into(),
            node_id,
            store,
            join_token: std::sync::Mutex::new(join_token),
            materials: MaterialStore::new(materials_dir),
            outbound: tokio::sync::Mutex::new(None),
            pending: tokio::sync::Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            spawn_serial: tokio::sync::Mutex::new(()),
            host: tokio::sync::Mutex::new(SessionHost::new(
                sessions_dir,
                0usize.max(max_sessions),
                // 存储上限随 4.5 的策略落地；0 = 暂不限。
                0,
                Arc::new(EchoOnlyFactory),
            )),
            router: Arc::new(std::sync::Mutex::new(None)),
            // 能力清单待节点侧 agent 配置落地后填充（任务 2.4 的运行时一半）；
            // 现在如实上报「什么都没有」，而不是编一份看起来丰富的清单。
            manifest: CapabilityManifest::default(),
            tuning: LinkTuning::default(),
            // 默认 30 天，与 `config::DEFAULT_LOG_RETENTION_DAYS` 一致；真实值由
            // `with_retention_days` 从节点配置灌进来。
            retention_days: crate::config::DEFAULT_LOG_RETENTION_DAYS,
        }
    }

    /// 覆盖调参（测试用）。
    pub fn with_tuning(mut self, tuning: LinkTuning) -> Self {
        self.tuning = tuning;
        self
    }

    /// 指定本地日志保留天数（4.4）。
    pub fn with_retention_days(mut self, days: u32) -> Self {
        self.retention_days = days;
        self
    }

    /// 指定握手要上报的**能力清单**（由节点配置构建，见 `NodeConfig::manifest`）。
    ///
    /// 不给就是空清单——如实表示"什么都没配"，而不是编一份看起来丰富的。
    pub fn with_manifest(mut self, manifest: CapabilityManifest) -> Self {
        self.manifest = manifest;
        self
    }

    /// 装上真实执行体工厂，并共享控制面 router 端点的槽（6.3 / 7.2）。
    ///
    /// 工厂默认是 `echo`（连通性验证用）。装真工厂要拿宿主的锁，因此是 async 的
    /// ——启动路径本来就是 async，不引入额外阻塞。
    pub async fn with_body_factory(
        mut self,
        factory: Arc<dyn BodyFactory + Send + Sync>,
        router: RouterCell,
    ) -> Self {
        self.host.lock().await.set_factory(factory);
        self.router = router;
        self
    }

    /// 记下（或撤回）控制面告知的 router 端点。
    fn note_router_endpoint(&self, url: Option<String>, token: Option<String>) {
        let mut slot = self
            .router
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = match url {
            Some(url) => Some(RouterEndpoint { url, token }),
            None => None,
        };
    }

    /// 组装本次握手的凭据：**还没用过的** join token 优先（那是操作者要求配对），
    /// 否则用已存长期凭据。
    fn auth(&self) -> Result<NodeAuth, NodeError> {
        let token = self
            .join_token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(token) = token {
            return Ok(NodeAuth::JoinToken { token });
        }
        match self.store.load_credential()? {
            Some(secret) => Ok(NodeAuth::Credential { secret }),
            None => Err(NodeError::unpaired(format!(
                "节点 {} 没有凭据：首次接入需要 --join-token <token>（在主控侧签发）",
                self.node_id
            ))),
        }
    }

    /// 一次拨号 + 握手。
    pub async fn attempt(&self) -> Attempt {
        if self.control_plane.starts_with("wss://") {
            return Attempt::Fatal(NodeError::link_unavailable(format!(
                "{} 是 wss://，节点侧 TLS 尚未实现；请让节点连本地反代的 ws://（TLS 由部署方终止）",
                self.control_plane
            )));
        }
        if !self.control_plane.starts_with("ws://") {
            return Attempt::Fatal(NodeError::config(format!(
                "control_plane {:?} 不是 websocket 端点",
                self.control_plane
            )));
        }

        let auth = match self.auth() {
            Ok(auth) => auth,
            Err(e) => return Attempt::Fatal(e),
        };
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            node_id: self.node_id.to_string(),
            auth,
            manifest: self.manifest.clone(),
        };
        let payload = match serde_json::to_string(&hello) {
            Ok(p) => p,
            Err(e) => {
                return Attempt::Fatal(NodeError::link_unavailable(format!(
                    "无法序列化握手报文：{e}"
                )));
            }
        };

        let (mut ws, _resp) = match tokio_tungstenite::connect_async(&self.control_plane).await {
            Ok(pair) => pair,
            Err(e) => return Attempt::Transient(format!("拨号 {} 失败：{e}", self.control_plane)),
        };
        if let Err(e) = ws.send(Message::Text(payload.into())).await {
            return Attempt::Transient(format!("发送握手失败：{e}"));
        }

        let ack = match tokio::time::timeout(self.tuning.handshake_timeout, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<HelloAck>(&text) {
                Ok(ack) => ack,
                Err(e) => {
                    return Attempt::Fatal(NodeError::link_unavailable(format!(
                        "主控握手应答无法解析：{e}"
                    )));
                }
            },
            Ok(Some(Ok(other))) => {
                return Attempt::Transient(format!("主控在握手阶段发来非文本帧：{other:?}"));
            }
            Ok(Some(Err(e))) => return Attempt::Transient(format!("握手期间链路错误：{e}")),
            Ok(None) => return Attempt::Transient("主控在应答握手前关闭了链路".into()),
            Err(_) => {
                return Attempt::Transient(format!(
                    "等待握手应答超过 {:?}",
                    self.tuning.handshake_timeout
                ));
            }
        };

        // 控制面可能在应答里告知它的 router 端点（7.2）：接受与否都要按**它说的**
        // 更新槽位；拒绝时清空（旧连接告知的地址不该在断链后继续被使用）。
        let HelloAck {
            outcome,
            router_url,
            router_token,
            ..
        } = ack;
        match outcome {
            HelloOutcome::Accepted { credential } => {
                if let Some(secret) = credential {
                    // 凭据只在配对时出现一次：先落盘再声称成功。
                    if let Err(e) = self.store.save_credential(&secret) {
                        return Attempt::Fatal(e);
                    }
                    // **token 用掉了就作废**：它是一次性的，留着只会在下一次重连时
                    // 换来一个永久拒绝（`join_token_consumed`），把一次链路抖动
                    // 变成节点永久退出。此后一律用刚落盘的长期凭据。
                    if self
                        .join_token
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                        .is_some()
                    {
                        eprintln!(
                            "sebas-node: 配对完成，一次性 token 已作废；此后使用状态目录里的长期凭据"
                        );
                    }
                }
                self.note_router_endpoint(router_url, router_token);
                Attempt::Connected(ws)
            }
            HelloOutcome::Rejected { code, cause } => {
                self.note_router_endpoint(None, None);
                if code.is_permanent() {
                    Attempt::Fatal(NodeError::link_unavailable(format!(
                        "主控拒绝接入（{}）：{cause}",
                        code.as_str()
                    )))
                } else {
                    Attempt::Transient(format!("主控暂时不可用（{}）：{cause}", code.as_str()))
                }
            }
        }
    }

    /// 向**控制面**发一次请求（节点 → 控制面方向）。链路不在线 → 如实失败。
    pub async fn request_from_node(
        &self,
        op: SessionOp,
        timeout: Duration,
    ) -> Result<SessionResult, NodeError> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let frame = Frame::Request { id, op };
        let json = serde_json::to_string(&frame)
            .map_err(|e| NodeError::link_unavailable(format!("无法序列化请求：{e}")))?;
        let sent = {
            let guard = self.outbound.lock().await;
            match guard.as_ref() {
                Some(tx) => tx.send(Message::Text(json.into())).is_ok(),
                None => false,
            }
        };
        if !sent {
            self.pending.lock().await.remove(&id);
            return Err(NodeError::link_unavailable("链路不在线，请求未发出"));
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => {
                self.pending.lock().await.remove(&id);
                Err(NodeError::link_unavailable("链路在等待应答时断开"))
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(NodeError::link_unavailable(format!(
                    "等待控制面应答超时（{timeout:?}）"
                )))
            }
        }
    }

    /// 会话创建前的材料处理：拉取当前版本并交给宿主钉住。
    ///
    /// 控制面**没配材料**（如实拒绝）或拉取失败时：不钉版本、如实记日志，会话照常建立
    /// ——材料的缺省不应把会话挡在门外，但也不能假装钉住了什么。
    async fn pin_materials_for_spawn(&self, agent_kind: Option<&str>) {
        let result = match self
            .request_from_node(
                SessionOp::FetchMaterials { version: None },
                MATERIALS_TIMEOUT,
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                eprintln!("sebas-node: 材料拉取失败（本次不钉版本）：{e}");
                return;
            }
        };

        match result {
            SessionResult::Materials { version, files } => {
                let pinned = match self.materials.existing(&version) {
                    Some(pinned) => pinned,
                    None => match self.materials.install(&version, &files) {
                        Ok(pinned) => pinned,
                        Err(e) => {
                            eprintln!("sebas-node: 材料落盘失败（本次不钉版本）：{e}");
                            return;
                        }
                    },
                };
                let body = agent_kind.unwrap_or("echo");
                match self.materials.place_for(body, &pinned) {
                    Ok(dir) => {
                        self.host
                            .lock()
                            .await
                            .set_pending_materials(pinned.version.clone(), dir);
                    }
                    Err(e) => {
                        // 落点没约定：**不落材料也不假装钉住**（7.5）。
                        eprintln!("sebas-node: {e}");
                    }
                }
            }
            SessionResult::Rejected { cause, .. } => {
                eprintln!("sebas-node: 控制面未提供操作者级材料（{cause}）");
            }
            other => {
                eprintln!("sebas-node: 材料拉取得到非预期应答：{other:?}");
            }
        }
    }

    /// 持续维持链路：瞬时失败退避重试，永久失败如实退出。
    ///
    /// 除链路循环外还挂一个**保留期回收**巡检（4.4）：它是节点自主策略，与链路
    /// 生死无关（离线时回收照做，水位线在 outbox 里等重连上报），因此在这里起独立
    /// 任务；`run` 返回时一并收走，不留孤儿任务。
    pub async fn run(self: &Arc<Self>) -> Result<(), NodeError> {
        // 守卫管生命周期：`run` 无论正常返回还是被 abort，巡检都会随之收走
        // （见 [`AbortOnDrop`]——它同时保住套接字能被关掉）。
        let _sweeper = AbortOnDrop({
            let me = Arc::clone(self);
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(me.tuning.retention_sweep);
                // 巡检错过一次不该连续补跑：按天计的策略，晚一小时无所谓。
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    let swept = me
                        .host
                        .lock()
                        .await
                        .sweep_retention(me.retention_days, crate::log::now_unix());
                    if !swept.is_empty() {
                        eprintln!(
                            "sebas-node: 已按 {} 天保留期回收旧日志，涉及 {} 个会话：{}",
                            me.retention_days,
                            swept.len(),
                            swept.join(", ")
                        );
                    }
                }
            })
        });
        self.run_link_forever().await
    }

    /// 链路主循环（握手 → 读循环 → 断链退避重连），不含保留期巡检。
    async fn run_link_forever(self: &Arc<Self>) -> Result<(), NodeError> {
        let mut backoff = self.tuning.backoff_start;
        loop {
            match self.attempt().await {
                Attempt::Fatal(e) => return Err(e),
                Attempt::Transient(cause) => {
                    eprintln!(
                        "sebas-node: {cause}；{:?} 后退避重试",
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = next_backoff(backoff, self.tuning.backoff_cap);
                }
                Attempt::Connected(ws) => {
                    // 握手成功：退避复位。
                    backoff = self.tuning.backoff_start;
                    eprintln!(
                        "sebas-node: 已连上主控 {}（node id {}）",
                        self.control_plane, self.node_id
                    );
                    // 链路是**双向**的：控制面发来操作（我们应答），节点也会主动发
                    // 请求（拉材料）。因此把套接字拆成读写两半，写半交给一个任务。
                    let (mut sink, mut stream) = futures_util::StreamExt::split(ws);
                    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
                    let writer = tokio::spawn(async move {
                        while let Some(msg) = out_rx.recv().await {
                            if sink.send(msg).await.is_err() {
                                break;
                            }
                        }
                        let _ = sink.close().await;
                    });
                    *self.outbound.lock().await = Some(out_tx.clone());

                    // 读循环：会话操作 → 宿主 → 应答；宿主产出的合并批 → 上行事件。
                    let mut ticker = tokio::time::interval(Duration::from_millis(50));
                    'link: loop {
                        tokio::select! {
                            incoming = futures_util::StreamExt::next(&mut stream) => {
                                match incoming {
                                    Some(Ok(Message::Close(_))) | None => break 'link,
                                    Some(Ok(Message::Ping(payload))) => {
                                        let _ = out_tx.send(Message::Pong(payload));
                                    }
                                    Some(Ok(Message::Text(text))) => {
                                        match serde_json::from_str::<Frame>(&text) {
                                            Ok(Frame::Request { id, op }) => {
                                                // **交给独立任务**：材料拉取要在本连接上
                                                // 等应答，而应答正是这条读循环收的——就地
                                                // await 会死锁。应答按 id 关联，乱序无妨。
                                                let me = Arc::clone(self);
                                                let out = out_tx.clone();
                                                let is_spawn =
                                                    matches!(op, SessionOp::Spawn { .. });
                                                tokio::spawn(async move {
                                                    // Spawn 路径串行：「拉材料 → 钉版本 →
                                                    // 建会话」必须原子，否则并发 Spawn 会
                                                    // 让材料槽串味。
                                                    let _guard = if is_spawn {
                                                        Some(me.spawn_serial.lock().await)
                                                    } else {
                                                        None
                                                    };
                                                    if let SessionOp::Spawn { agent_kind, .. } = &op {
                                                        me.pin_materials_for_spawn(
                                                            agent_kind.as_deref(),
                                                        )
                                                        .await;
                                                    }
                                                    let response =
                                                        me.host.lock().await.handle(id, op);
                                                    if let Ok(json) =
                                                        serde_json::to_string(&response)
                                                    {
                                                        let _ = out.send(Message::Text(json.into()));
                                                    }
                                                });
                                            }
                                            // 节点主动请求的应答：按 id 回填等待者。
                                            Ok(Frame::Response { id, result }) => {
                                                let waiter = self.pending.lock().await.remove(&id);
                                                if let Some(tx) = waiter {
                                                    let _ = tx.send(result);
                                                } else {
                                                    eprintln!(
                                                        "sebas-node: 控制面应答 id={id} 无等待者（可能已超时）"
                                                    );
                                                }
                                            }
                                            // 控制面不该发别的帧：如实记录，但不断链
                                            // （链路是双方共用的，误帧不该毁掉会话）。
                                            Ok(other) => {
                                                eprintln!("sebas-node: 忽略非请求帧：{other:?}");
                                            }
                                            Err(e) => {
                                                eprintln!("sebas-node: 帧无法解析（已忽略）：{e}");
                                            }
                                        }
                                    }
                                    Some(Ok(_)) => { /* 二进制帧暂不用 */ }
                                    Some(Err(e)) => {
                                        eprintln!("sebas-node: 链路错误：{e}");
                                        break 'link;
                                    }
                                }
                            }
                            _ = ticker.tick() => {
                                let events = self
                                    .host
                                    .lock()
                                    .await
                                    .drain_events(Instant::now());
                                for event in events {
                                    let frame = Frame::Event { event };
                                    match serde_json::to_string(&frame) {
                                        Ok(json) => {
                                            if out_tx.send(Message::Text(json.into())).is_err() {
                                                break 'link;
                                            }
                                        }
                                        Err(e) => eprintln!("sebas-node: 事件无法序列化：{e}"),
                                    }
                                }
                            }
                        }
                    }
                    *self.outbound.lock().await = None;
                    // 断链即撤回 router 端点：在飞执行不受影响，但**新会话**不该用
                    // 一个已经可能不可达的地址（重连后由新的握手重新告知）。
                    self.note_router_endpoint(None, None);
                    {
                        // 待决请求如实失败（丢弃 sender → 等待者收到 Err）。
                        self.pending.lock().await.clear();
                    }
                    writer.abort();
                    eprintln!("sebas-node: 链路已断开，重连中");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn store() -> (tempfile::TempDir, IdentityStore, NodeId) {
        let dir = tempfile::tempdir().unwrap();
        let store = IdentityStore::new(dir.path().join("state"));
        let id = store.load_or_create_id(Some("test-node")).unwrap();
        (dir, store, id)
    }

    /// 起一个一次性假主控：收一个握手帧，按脚本回一个应答。
    async fn fake_control_plane(
        reply: Option<HelloAck>,
    ) -> (u16, tokio::task::JoinHandle<Option<Hello>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.ok()?;
            let mut ws = tokio_tungstenite::accept_async(stream).await.ok()?;
            let hello = match ws.next().await {
                Some(Ok(Message::Text(text))) => serde_json::from_str::<Hello>(&text).ok(),
                _ => None,
            };
            if let Some(reply) = reply {
                let _ = ws
                    .send(Message::Text(serde_json::to_string(&reply).unwrap().into()))
                    .await;
                // 让客户端读完应答再关。
                let _ = ws.next().await;
            }
            hello
        });
        (port, handle)
    }

    fn tuning() -> LinkTuning {
        LinkTuning {
            backoff_start: Duration::from_millis(5),
            backoff_cap: Duration::from_millis(20),
            handshake_timeout: Duration::from_secs(5),
            // 单测不该被小时级巡检打扰（要测巡检本身就用 host 级单测）。
            retention_sweep: Duration::from_secs(3600),
        }
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let cap = Duration::from_secs(60);
        assert_eq!(
            next_backoff(Duration::from_secs(1), cap),
            Duration::from_secs(2)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(32), cap),
            Duration::from_secs(60),
            "翻倍超过上限应封顶"
        );
        assert_eq!(
            next_backoff(Duration::from_secs(60), cap),
            Duration::from_secs(60),
            "已到上限后保持"
        );
    }

    /// 一次性 token **用过就作废**：否则第一次断链重连会拿它再握手，主控回
    /// `join_token_consumed`（永久拒绝），节点就此永久退出——一次链路抖动被放大成
    /// 节点下线。这个回归由进程级 e2e 抓出来（杀主控重启后节点退出 75）。
    #[tokio::test]
    async fn a_spent_join_token_is_retired_in_favour_of_the_credential() {
        let (port, server) = fake_control_plane(Some(sebas_node_link::accepted(Some(
            "issued-secret".into(),
        ))))
        .await;
        let (_tmp, store, id) = store();
        let client = LinkClient::new(
            format!("ws://127.0.0.1:{port}"),
            id,
            store.clone(),
            Some("join-token".into()),
            _tmp.path().join("sessions"),
            _tmp.path().join("materials"),
            4,
        )
        .with_tuning(tuning());

        match client.attempt().await {
            Attempt::Connected(_) => {}
            other => panic!("首次配对应成功，实际 {}", describe(&other)),
        }
        let hello = server.await.unwrap().expect("服务端应收到合法握手");
        assert!(matches!(hello.auth, NodeAuth::JoinToken { .. }));

        // 第二次握手必须改用长期凭据——token 已经花掉了。
        match client.auth().unwrap() {
            NodeAuth::Credential { secret } => assert_eq!(secret, "issued-secret"),
            NodeAuth::JoinToken { .. } => {
                panic!("配对后仍在复用一次性 token：断链重连会被永久拒绝")
            }
        }
    }

    #[tokio::test]
    async fn pairing_stores_the_credential_and_connects() {
        let (port, server) = fake_control_plane(Some(sebas_node_link::accepted(Some(
            "issued-secret".into(),
        ))))
        .await;
        let (_tmp, store, id) = store();
        let client = LinkClient::new(
            format!("ws://127.0.0.1:{port}"),
            id,
            store.clone(),
            Some("join-token".into()),
            _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
            4,
        )
        .with_tuning(tuning());

        match client.attempt().await {
            Attempt::Connected(_) => {}
            Attempt::Fatal(e) => panic!("不应致命失败：{e}"),
            Attempt::Transient(c) => panic!("不应瞬时失败：{c}"),
        }
        // 凭据必须已落盘（重启后靠它接入）。
        assert_eq!(
            store.load_credential().unwrap().as_deref(),
            Some("issued-secret")
        );
        // 服务端确实收到了 join token 形式的握手。
        let hello = server.await.unwrap().expect("服务端应收到合法握手");
        assert_eq!(hello.node_id, "test-node");
        assert!(matches!(hello.auth, NodeAuth::JoinToken { .. }));
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn accepted_without_credential_keeps_existing_credential() {
        let (port, _server) = fake_control_plane(Some(sebas_node_link::accepted(None))).await;
        let (_tmp, store, id) = store();
        store.save_credential("already-paired").unwrap();
        let client = LinkClient::new(
            format!("ws://127.0.0.1:{port}"),
            id,
            store.clone(),
            None,
            _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
            4,
        )
        .with_tuning(tuning());

        match client.attempt().await {
            Attempt::Connected(_) => {}
            other => panic!("应连上，实际 {}", describe(&other)),
        }
        assert_eq!(
            store.load_credential().unwrap().as_deref(),
            Some("already-paired")
        );
    }

    #[tokio::test]
    async fn permanent_rejection_is_fatal_and_not_retried() {
        let (port, _server) = fake_control_plane(Some(sebas_node_link::rejected(
            PROTOCOL_VERSION,
            sebas_node_link::RejectCode::CredentialRevoked,
            "revoked by operator",
        )))
        .await;
        let (_tmp, store, id) = store();
        store.save_credential("stale").unwrap();
        let client = LinkClient::new(
                format!("ws://127.0.0.1:{port}"),
                id,
                store,
                None,
                _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
                4,
            )
        .with_tuning(tuning());

        match client.attempt().await {
            Attempt::Fatal(e) => {
                let msg = e.to_string();
                assert!(msg.contains("credential_revoked"), "{msg}");
                assert!(msg.contains("revoked by operator"), "{msg}");
            }
            other => panic!("永久拒绝应致命，实际 {}", describe(&other)),
        }
    }

    #[tokio::test]
    async fn version_mismatch_is_reported_with_both_versions() {
        let (port, _server) = fake_control_plane(Some(sebas_node_link::rejected(
            PROTOCOL_VERSION + 7,
            sebas_node_link::RejectCode::ProtocolVersionUnsupported,
            format!("主控支持 {PROTOCOL_VERSION}，节点为 {}", PROTOCOL_VERSION + 7),
        )))
        .await;
        let (_tmp, store, id) = store();
        store.save_credential("s").unwrap();
        let client = LinkClient::new(
                format!("ws://127.0.0.1:{port}"),
                id,
                store,
                None,
                _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
                4,
            )
        .with_tuning(tuning());

        match client.attempt().await {
            Attempt::Fatal(e) => {
                let msg = e.to_string();
                assert!(msg.contains("protocol_version_unsupported"), "{msg}");
                assert!(msg.contains(&(PROTOCOL_VERSION + 7).to_string()), "{msg}");
            }
            other => panic!("版本不兼容应致命，实际 {}", describe(&other)),
        }
    }

    #[tokio::test]
    async fn unreachable_control_plane_is_transient() {
        // 先占一个端口再释放，得到一个几乎必然拒绝连接的地址。
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let (_tmp, store, id) = store();
        store.save_credential("s").unwrap();
        let client = LinkClient::new(
                format!("ws://127.0.0.1:{port}"),
                id,
                store,
                None,
                _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
                4,
            )
        .with_tuning(tuning());

        match client.attempt().await {
            Attempt::Transient(cause) => assert!(cause.contains("拨号"), "{cause}"),
            other => panic!("连不上应瞬时失败，实际 {}", describe(&other)),
        }
    }

    #[tokio::test]
    async fn server_closing_before_ack_is_transient() {
        let (port, _server) = fake_control_plane(None).await;
        let (_tmp, store, id) = store();
        store.save_credential("s").unwrap();
        let client = LinkClient::new(
                format!("ws://127.0.0.1:{port}"),
                id,
                store,
                None,
                _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
                4,
            )
        .with_tuning(tuning());

        match client.attempt().await {
            // 对端未应答就断开：可能表现为「关闭」，也可能是协议级「链路错误」
            // （TCP reset 不带 close 握手）。两者都必须是**瞬时**失败，不能致命。
            Attempt::Transient(cause) => assert!(!cause.is_empty(), "瞬时失败也要有成因"),
            other => panic!("未应答即关闭应瞬时失败，实际 {}", describe(&other)),
        }
    }

    #[tokio::test]
    async fn wss_is_refused_honestly_instead_of_pretending() {
        let (_tmp, store, id) = store();
        store.save_credential("s").unwrap();
        let client = LinkClient::new(
            "wss://control.example/ws",
            id,
            store,
            None,
            _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
            4,
        );

        match client.attempt().await {
            Attempt::Fatal(e) => {
                let msg = e.to_string();
                assert!(msg.contains("wss://"), "{msg}");
                assert!(msg.contains("尚未实现"), "{msg}");
            }
            other => panic!("wss 应如实拒绝，实际 {}", describe(&other)),
        }
    }

    #[tokio::test]
    async fn missing_credential_without_token_is_fatal() {
        let (_tmp, store, id) = store();
        let client = LinkClient::new(
            "ws://127.0.0.1:9",
            id,
            store,
            None,
            _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
            4,
        );
        match client.attempt().await {
            Attempt::Fatal(e) => assert!(e.to_string().contains("未配对"), "{e}"),
            other => panic!("无凭据应致命，实际 {}", describe(&other)),
        }
    }

    #[tokio::test]
    async fn run_reconnects_after_the_link_drops() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // 假主控：接受两次连接。第一条应答后立即关闭（制造断线），第二条保持。
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&accepted);
        tokio::spawn(async move {
            for _ in 0..2 {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let nth = seen.fetch_add(1, Ordering::SeqCst) + 1;
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    continue;
                };
                let _ = ws.next().await; // 读 Hello
                let ack = sebas_node_link::accepted(None);
                let _ = ws
                    .send(Message::Text(serde_json::to_string(&ack).unwrap().into()))
                    .await;
                if nth == 1 {
                    // 立刻断开，逼客户端重连。
                    let _ = ws.close(None).await;
                }
            }
        });

        let (_tmp, store, id) = store();
        store.save_credential("s").unwrap();
        let client = Arc::new(
            LinkClient::new(format!("ws://127.0.0.1:{port}"), id, store, None,
            _tmp.path().join("sessions"),
                _tmp.path().join("materials"),
            4,
        )
                .with_tuning(tuning()),
        );
        let runner = Arc::clone(&client);
        let handle = tokio::spawn(async move { runner.run().await });

        // 退避起点是 5ms，很快应出现第二条连接。
        let mut ok = false;
        for _ in 0..200 {
            if accepted.load(Ordering::SeqCst) >= 2 {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(ok, "断线后应自动重连（已接受 {} 次）", accepted.load(Ordering::SeqCst));
        handle.abort();
    }

    /// 保留期回收是节点自主的（4.4）：**不用主控下令**，节点按自己的保留期扫，
    /// 扫出来的水位线必须上行——否则控制面会一直等着一个永远补不上的缺口。
    ///
    /// 这条测试真的把链路跑起来：预置一段 40 天前的日志 → 巡检（20ms 周期）→
    /// 假主控在 Event 帧里收到 `Reclaimed`。
    #[tokio::test]
    async fn retention_sweep_reports_the_watermark_over_the_link() {
        let (tx, rx) = oneshot::channel::<(String, u64)>();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                return;
            };
            let _ = ws.next().await; // 读 Hello
            let ack = sebas_node_link::accepted(None);
            let _ = ws
                .send(Message::Text(serde_json::to_string(&ack).unwrap().into()))
                .await;
            let mut tx = Some(tx);
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let Ok(Frame::Event { event }) = serde_json::from_str::<Frame>(&text) else {
                    continue;
                };
                if let sebas_node_link::SessionEvent::Reclaimed {
                    session_id,
                    reclaimed_through_seq,
                } = event
                {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send((session_id, reclaimed_through_seq));
                    }
                }
            }
        });

        let (tmp, store, id) = store();
        store.save_credential("s").unwrap();
        let sessions = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        // 一个 40 天前的会话（默认保留期 30 天）。
        let day = 86_400i64;
        let now = crate::log::now_unix();
        let watermark = {
            let mut log = crate::log::SessionLog::open(&sessions, "s-old").unwrap();
            log.append_at("output", "aged-1", None, now - 40 * day)
                .unwrap();
            log.append_at("output", "aged-2", None, now - 39 * day)
                .unwrap()
                .seq
        };

        let client = Arc::new(
            LinkClient::new(
                format!("ws://127.0.0.1:{port}"),
                id,
                store,
                None,
                sessions,
                tmp.path().join("materials"),
                4,
            )
            .with_tuning(LinkTuning {
                // 巡检周期压到毫秒级，测试不必等一小时。
                retention_sweep: Duration::from_millis(20),
                ..tuning()
            }),
        );
        let runner = Arc::clone(&client);
        let handle = tokio::spawn(async move { runner.run().await });

        let reported = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .expect("10 秒内应收到回收水位线（节点自主巡检应无需主控催）")
            .expect("假主控应把水位线发回来");
        assert_eq!(reported, ("s-old".to_string(), watermark));
        handle.abort();
    }

    /// 测试断言的辅助：把 Attempt 描述成可读字符串。
    fn describe(attempt: &Attempt) -> String {
        match attempt {
            Attempt::Connected(_) => "Connected".into(),
            Attempt::Fatal(e) => format!("Fatal({e})"),
            Attempt::Transient(c) => format!("Transient({c})"),
        }
    }
}
