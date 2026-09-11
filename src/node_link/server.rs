//! 主控侧节点链路的**运行时**：接受节点入站连接、完成握手、维护在线态。
//!
//! 由 **core 角色托管**（设计 D13）：这条链路上跑的是会话级操作，而 core 是会话状态的
//! 单一权威、也是唯一 spawn 执行体的进程；远端节点本质上是「远程执行体宿主」。
//! 若改由 webui 托管，webui 会从「观察/驱动客户端」变成「中继」，且远程执行会额外
//! 依赖 webui 在线。
//!
//! 写者唯一性：注册表是**单写者**文件。因此服务端独占一份 [`NodeRegistry`]（在
//! `Mutex` 之内），并发连接共享同一份内存状态与写权——绝不每条连接各自 `open` 一次，
//! 否则并发的 `save()` 会互相覆盖。锁只在**注册表操作**期间短暂持有，绝不跨越
//! 网络读写（那会让一个慢节点卡住所有节点）。

use crate::node_link::Rejection;
use futures_util::SinkExt;
use crate::node_link::registry::{NodeRegistry, RegistryError};
use sebas_node_link::{Hello, NodeAuth, PROTOCOL_VERSION, RejectCode, validate_node_id};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, watch};
use tokio_tungstenite::tungstenite::Message;

/// 默认握手超时：超时即放弃该连接（不无限占着任务）。
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// 一次连接的处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handled {
    /// 成功接入，并在断开后标记离线。`paired` 为真表示本次完成了配对（签发了新凭据）。
    Accepted {
        /// 节点标识。
        node_id: String,
        /// 是否本次完成配对。
        paired: bool,
    },
    /// 协议层拒绝（节点收到同样的码与成因）。
    Rejected {
        /// 节点标识（报文合法但 id 非法时为 `None`）。
        node_id: Option<String>,
        /// 拒绝码。
        code: RejectCode,
        /// 成因。
        cause: String,
    },
    /// 报文层面不可用（非文本帧、非法 JSON、非法 id、WS 握手失败）。
    Malformed {
        /// 成因。
        cause: String,
    },
}

/// 连接**生命周期**观察者：节点接入/断开时被通知。
///
/// 与 [`crate::node_link::client::InboundHandler`] 的分工：那个处理节点**发来的
/// 请求**，这个只被告知"链路来过了/走了"。core 用它驱动远端会话投影（5.1/5.3）：
/// 接入时按节点的事实重建视图并开始消费事件流，断开时把该节点的会话如实标成
/// **暂时看不见**（不是终止——见 `fleet` 的三种结论）。
#[async_trait::async_trait]
pub trait ConnectionObserver: Send + Sync {
    /// 一条连接已被接受并登记在线（此刻 `live_connection` 已可用）。
    async fn connected(&self, node_id: &str, connection: Arc<crate::node_link::NodeConnection>);
    /// 一条连接已断开（在线表已移除、注册表已标离线）。
    ///
    /// `cause` 是链路终止的成因（`None` = 正常关闭）。
    async fn disconnected(&self, node_id: &str, cause: Option<String>);
}

/// 主控要在握手里告知节点的 **router 端点**（add-remote-execution-node 7.2）。
///
/// 节点配 `upstream = control-plane-router` 时，模型流量要指回主控的 router——而
/// 地址只有主控知道（节点可能在内网、可能经反代）。所以这个地址**由主控给出**：
/// 没启用内置 router 时是 `None`，节点据此如实拒绝，而不是猜一个地址去撞。
///
/// `token` 是主控自己 router 的**下游凭据**，不是 provider 密钥：节点仍然零
/// provider 凭据（这正是 7.2 要保住的性质）。
#[derive(Debug, Clone, Default)]
pub struct RouterEndpoint {
    /// router 的基地址（含 scheme，如 `http://127.0.0.1:8787`）。
    pub url: Option<String>,
    /// 访问该 router 的下游凭据（router 未要求鉴权时为 `None`）。
    pub token: Option<String>,
}

/// 节点链路服务端。
pub struct NodeLinkServer {
    listener: TcpListener,
    registry: Arc<Mutex<NodeRegistry>>,
    /// 在线连接（节点标识 → 控制面句柄）。
    live: Arc<Mutex<HashMap<String, Arc<crate::node_link::NodeConnection>>>>,
    /// 来向请求处理器（材料拉取等）；`None` = 不处理来向请求。
    handler: Option<Arc<dyn crate::node_link::client::InboundHandler>>,
    /// 连接生命周期观察者（远端会话投影）；`None` = 没人关心接入/断开。
    observer: Option<Arc<dyn ConnectionObserver>>,
    /// 握手时要告知节点的 router 端点（7.2）。
    router: RouterEndpoint,
    handshake_timeout: Duration,
}

impl NodeLinkServer {
    /// 绑定地址并打开注册表。
    pub async fn bind(addr: &str, registry_path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let registry = NodeRegistry::open(registry_path.into())
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Self::bind_with(addr, Arc::new(Mutex::new(registry))).await
    }

    /// 用**已有的**注册表写者句柄绑定。
    ///
    /// 生产路径用这个：托管进程（core）先打开注册表，再把**同一个**句柄交给监听与
    /// 管理入口。用 [`Self::bind`] 另开实例在测试里方便，但在生产里会造成两个内存
    /// 副本互相覆盖（单写者约定）。
    pub async fn bind_with(
        addr: &str,
        registry: Arc<Mutex<NodeRegistry>>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            listener,
            registry,
            live: Arc::new(Mutex::new(HashMap::new())),
            handler: None,
            observer: None,
            router: RouterEndpoint::default(),
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        })
    }

    /// 实际绑定到的地址（端口为 0 时用来取真实端口）。
    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// 注册表句柄：与监听共享**同一份写者状态**。
    ///
    /// 签发 join token、吊销凭据、查看在线态都必须走这里。**不要另开一个
    /// `NodeRegistry` 实例去写同一个文件**——两个实例各持一份内存副本，后写者会
    /// 覆盖先写者（单写者约定）。托管它的进程应把这个句柄分发给需要管理节点的
    /// 上层（如 webui 的服务页）。
    pub fn registry(&self) -> Arc<Mutex<NodeRegistry>> {
        Arc::clone(&self.registry)
    }

    /// 在线连接表（只读）：供上层按节点标识取连接句柄驱动会话。
    pub fn live_connections(
        &self,
    ) -> Arc<Mutex<HashMap<String, Arc<crate::node_link::NodeConnection>>>> {
        Arc::clone(&self.live)
    }

    /// 取某节点的在线连接（不在线 → `None`）。
    pub async fn live_connection(&self, node_id: &str) -> Option<Arc<crate::node_link::NodeConnection>> {
        self.live.lock().await.get(node_id).cloned()
    }

    /// 装配**来向请求处理器**（材料拉取必需）。
    pub fn with_inbound_handler(
        mut self,
        handler: Arc<dyn crate::node_link::client::InboundHandler>,
    ) -> Self {
        self.handler = Some(handler);
        self
    }

    /// 装配**连接生命周期观察者**（远端会话投影需要它）。
    pub fn with_observer(mut self, observer: Arc<dyn ConnectionObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// 装配**router 端点**：此后每次接受的握手都会把它告知节点（7.2）。
    pub fn with_router_endpoint(mut self, router: RouterEndpoint) -> Self {
        self.router = router;
        self
    }

    /// 覆盖握手超时（测试用）。
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// 接受并处理**一条**连接（测试与长驻循环共用）。
    pub async fn accept_one(&self) -> Result<Handled, RegistryError> {
        let (stream, _peer) = self
            .listener
            .accept()
            .await
            .map_err(|e| RegistryError::Io {
                cause: format!("accept 失败：{e}"),
            })?;
        handle_connection(
            stream,
            &self.registry,
            &self.live,
            self.handler.clone(),
            self.observer.clone(),
            self.router.clone(),
            self.handshake_timeout,
        )
        .await
    }

    /// 长驻循环：每条连接一个任务；收到关闭信号即停止接受新连接。
    ///
    /// 取 `&self`：这样托管方可以把它放进 `Arc`（既跑服务循环，又把
    /// `live_connection` 交给上层驱动会话）。发送端 drop 也视为关闭——否则 select
    /// 会因为 `changed()` 立刻返回 `Err` 而空转。
    pub async fn serve(&self, mut shutdown: watch::Receiver<bool>) -> Result<(), RegistryError> {
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
                accepted = self.listener.accept() => {
                    let (stream, peer) = accepted.map_err(|e| RegistryError::Io {
                        cause: format!("accept 失败：{e}"),
                    })?;
                    let registry = Arc::clone(&self.registry);
                    let live = Arc::clone(&self.live);
                    let handler = self.handler.clone();
                    let observer = self.observer.clone();
                    let router = self.router.clone();
                    let timeout = self.handshake_timeout;
                    tokio::spawn(async move {
                        match handle_connection(
                            stream, &registry, &live, handler, observer, router, timeout,
                        )
                        .await
                        {
                            Ok(Handled::Accepted { node_id, .. }) => {
                                eprintln!("node-link: 节点 {node_id} 已接入（{peer}）");
                            }
                            Ok(Handled::Rejected { node_id, code, cause }) => {
                                eprintln!(
                                    "node-link: 拒绝 {:?}（{}）：{cause}",
                                    node_id, code.as_str()
                                );
                            }
                            Ok(Handled::Malformed { cause }) => {
                                eprintln!("node-link: 报文不可用（{peer}）：{cause}");
                            }
                            Err(e) => eprintln!("node-link: 处理 {peer} 失败：{e}"),
                        }
                    });
                }
            }
        }
    }
}

/// 处理一条已接受的 TCP 连接：**WS 握手 → 协议握手 → 维持 → 断开时标记离线**。
pub async fn handle_connection(
    stream: TcpStream,
    registry: &Mutex<NodeRegistry>,
    // 在线连接表：键为节点标识，值为其控制面句柄（3.5 的一半）。
    live: &Mutex<HashMap<String, Arc<crate::node_link::NodeConnection>>>,
    // 来向请求处理器（材料拉取）；`None` = 不处理来向请求。
    handler: Option<Arc<dyn crate::node_link::client::InboundHandler>>,
    // 连接生命周期观察者；`None` = 不上报接入/断开。
    observer: Option<Arc<dyn ConnectionObserver>>,
    // 要告知节点的 router 端点（7.2）。
    router_endpoint: RouterEndpoint,
    timeout: Duration,
) -> Result<Handled, RegistryError> {
    let mut ws = match tokio::time::timeout(timeout, tokio_tungstenite::accept_async(stream)).await {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            return Ok(Handled::Malformed {
                cause: format!("websocket 握手失败：{e}"),
            });
        }
        Err(_) => {
            return Ok(Handled::Malformed {
                cause: "websocket 握手超时".into(),
            });
        }
    };

    // 第一个帧必须是 Hello 文本。
    let hello = match tokio::time::timeout(timeout, futures_util::StreamExt::next(&mut ws)).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<Hello>(&text) {
            Ok(hello) => hello,
            Err(e) => {
                return Ok(Handled::Malformed {
                    cause: format!("Hello 无法解析：{e}"),
                });
            }
        },
        Ok(Some(Ok(other))) => {
            return Ok(Handled::Malformed {
                cause: format!("第一个帧不是文本：{other:?}"),
            });
        }
        Ok(Some(Err(e))) => {
            return Ok(Handled::Malformed {
                cause: format!("读取 Hello 时链路错误：{e}"),
            });
        }
        Ok(None) => {
            return Ok(Handled::Malformed {
                cause: "节点在发送 Hello 前就断开".into(),
            });
        }
        Err(_) => {
            return Ok(Handled::Malformed {
                cause: "等待 Hello 超时".into(),
            });
        }
    };

    // 握手带来的能力清单：登记到连接上，供上层只提供可达选项。
    let manifest = hello.manifest.clone();

    // 节点标识必须符合**双方共识**的规则（契约 crate 里的同一函数）。
    let node_id = match validate_node_id(&hello.node_id) {
        Ok(id) => id.to_string(),
        Err(cause) => {
            let rejection = Rejection::new(RejectCode::MalformedHello, cause);
            send_ack(&mut ws, rejection.to_ack()).await;
            return Ok(Handled::Rejected {
                node_id: None,
                code: rejection.code,
                cause: rejection.cause,
            });
        }
    };

    // 协议版本：不兼容就如实拒绝并同时报出两版本，不半工作。
    if hello.protocol_version != PROTOCOL_VERSION {
        let rejection = Rejection::new(
            RejectCode::ProtocolVersionUnsupported,
            format!(
                "主控支持 {PROTOCOL_VERSION}，节点为 {}",
                hello.protocol_version
            ),
        );
        send_ack(&mut ws, rejection.to_ack()).await;
        return Ok(Handled::Rejected {
            node_id: Some(node_id),
            code: rejection.code,
            cause: rejection.cause,
        });
    }

    let now = now_unix();
    // 注册表操作短暂持锁；**绝不跨网络**。
    let auth_result = {
        let mut reg = registry.lock().await;
        match &hello.auth {
            NodeAuth::JoinToken { token } => reg
                .consume_join_token(token, &node_id, now)
                .and_then(|()| reg.pair(&node_id, now))
                .map(|credential| (Some(credential), true)),
            NodeAuth::Credential { secret } => reg
                .authenticate(&node_id, secret)
                .map(|_status| (None, false)),
        }
    };

    let (credential, paired) = match auth_result {
        Ok(pair) => pair,
        Err(rejection) => {
            send_ack(&mut ws, rejection.to_ack()).await;
            return Ok(Handled::Rejected {
                node_id: Some(node_id),
                code: rejection.code,
                cause: rejection.cause,
            });
        }
    };

    // 告知接受（配对时把新凭据随应答交付——它只出现这一次），再登记在线。
    let ack = sebas_node_link::accepted_with_router(
        credential,
        router_endpoint.url.clone(),
        router_endpoint.token.clone(),
    );
    if let Err(e) = ws
        .send(Message::Text(
            serde_json::to_string(&ack)
                .unwrap_or_default()
                .into(),
        ))
        .await
    {
        return Ok(Handled::Malformed {
            cause: format!("发送应答失败：{e}"),
        });
    }
    {
        let mut reg = registry.lock().await;
        reg.mark_online(&node_id, now)?;
    }

    // 握手完成后的链路交给控制面连接句柄：请求/应答按 id 关联、事件流广播、
    // 断开时先把待决请求如实失败掉（3.5 的控制面一半）。
    let (sink, stream) = futures_util::StreamExt::split(ws);
    let connection = crate::node_link::NodeConnection::adopt_split(
        node_id.clone(),
        manifest,
        handler,
        stream,
        sink,
    );
    live.lock().await.insert(node_id.clone(), Arc::clone(&connection));
    // 先登记再通知：观察者拿到的句柄必须已经能用（它要立刻 ListSessions/订阅）。
    if let Some(observer) = &observer {
        observer
            .connected(&node_id, Arc::clone(&connection))
            .await;
    }
    // 等这条链路自然结束（节点离开 / 网络断 / 被关闭）。
    connection.wait_closed().await;

    // **先标离线，再移出 live 表**：顺序反了会出现一个窗口——live 表里已经没有它，
    // 但注册表还说它是 Online，此刻重连会被判 NodeIdConflict（永久拒绝），节点就此
    // 不再重试。反过来则「live 表里没有」蕴含「注册表已离线」，重连必然被接受。
    {
        let mut reg = registry.lock().await;
        reg.mark_offline(&node_id)?;
    }
    live.lock().await.remove(&node_id);
    if let Some(observer) = &observer {
        observer
            .disconnected(&node_id, connection.close_reason().await)
            .await;
    }
    Ok(Handled::Accepted { node_id, paired })
}

/// 发送应答，失败只记录（调用方随后都会关闭连接）。
async fn send_ack<S>(ws: &mut S, ack: sebas_node_link::HelloAck)
where
    S: futures_util::Sink<Message> + Unpin,
{
    if let Ok(text) = serde_json::to_string(&ack) {
        let _ = ws.send(Message::Text(text.into())).await;
    }
}

/// 当前 unix 秒。
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_link::NodeStatus;
    use futures_util::{SinkExt, StreamExt};
    use sebas_node_link::{CapabilityManifest, HelloAck, HelloOutcome};
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    /// 起一个服务端，并把**服务端自己那份注册表句柄**交给测试。
    ///
    /// 关键：准备数据（签发 token / 配对 / 吊销）与断言都必须走同一个句柄。
    /// 另开一个 `NodeRegistry` 实例会各持一份内存副本 —— 既看不到对方的写入，
    /// 一写还会互相覆盖（这正是本模块要守住单写者约定的原因）。
    async fn server() -> (tempfile::TempDir, String, Arc<Mutex<NodeRegistry>>, NodeLinkServer) {
        let dir = tempfile::tempdir().unwrap();
        let srv = NodeLinkServer::bind("127.0.0.1:0", dir.path().join("nodes.json"))
            .await
            .unwrap();
        let registry = srv.registry();
        let url = format!("ws://{}", srv.local_addr().unwrap());
        (dir, url, registry, srv)
    }

    fn hello(id: &str, auth: NodeAuth) -> Hello {
        Hello {
            protocol_version: PROTOCOL_VERSION,
            node_id: id.into(),
            auth,
            manifest: CapabilityManifest::default(),
        }
    }

    async fn send_hello(url: &str, hello: &Hello) -> HelloAck {
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        ws.send(ClientMessage::Text(serde_json::to_string(hello).unwrap().into()))
            .await
            .unwrap();
        let text = match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => t,
            other => panic!("未收到应答：{other:?}"),
        };
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn pairing_consumes_token_issues_credential_and_marks_online() {
        let (_tmp, url, registry, srv) = server().await;
        let token = registry
            .lock()
            .await
            .issue_join_token(now_unix(), 600)
            .unwrap();

        let handle = tokio::spawn(async move { srv.accept_one().await });
        let ack = send_hello(&url, &hello("dev-box", NodeAuth::JoinToken { token: token.clone() })).await;

        let secret = match ack.outcome {
            HelloOutcome::Accepted { credential } => credential.expect("配对必须交付新凭据"),
            other => panic!("应被接受，实际 {other:?}"),
        };
        assert_eq!(secret.len(), 64);
        assert_eq!(
            handle.await.unwrap().unwrap(),
            Handled::Accepted { node_id: "dev-box".into(), paired: true }
        );

        // 断开后：新凭据有效、节点回到离线、token 已被消费。
        let mut reg = registry.lock().await;
        assert!(reg.authenticate("dev-box", &secret).is_ok());
        assert_eq!(reg.node("dev-box").unwrap().status(), NodeStatus::Offline);
        assert!(reg.node("dev-box").unwrap().last_seen_unix().is_some());
        assert_eq!(
            reg.consume_join_token(&token, "dev-box", now_unix()).unwrap_err().code,
            RejectCode::JoinTokenConsumed
        );
    }

    #[tokio::test]
    async fn existing_credential_connects_without_new_credential() {
        let (_tmp, url, registry, srv) = server().await;
        let secret = registry.lock().await.pair("dev-box", now_unix()).unwrap();

        let handle = tokio::spawn(async move { srv.accept_one().await });
        let ack = send_hello(
            &url,
            &hello("dev-box", NodeAuth::Credential { secret: secret.clone() }),
        )
        .await;
        assert!(
            matches!(ack.outcome, HelloOutcome::Accepted { credential: None }),
            "已配对节点不应再收到凭据"
        );
        assert_eq!(
            handle.await.unwrap().unwrap(),
            Handled::Accepted { node_id: "dev-box".into(), paired: false }
        );
        // 连接结束后回到离线态，凭据仍然有效。
        let reg = registry.lock().await;
        assert_eq!(reg.authenticate("dev-box", &secret).unwrap(), NodeStatus::Offline);
    }

    #[tokio::test]
    async fn wrong_credential_is_rejected() {
        let (_tmp, url, registry, srv) = server().await;
        let _ = registry.lock().await.pair("dev-box", now_unix()).unwrap();

        let handle = tokio::spawn(async move { srv.accept_one().await });
        let ack = send_hello(
            &url,
            &hello("dev-box", NodeAuth::Credential { secret: "wrong".into() }),
        )
        .await;
        let (code, cause) = sebas_node_link::rejection_of(&ack).unwrap();
        assert_eq!(code, RejectCode::CredentialInvalid);
        assert!(cause.contains("未注册") || cause.contains("不正确"), "{cause}");
        let _ = handle.await;
        // 拒绝不影响既有凭据。
        assert!(registry.lock().await.node("dev-box").is_some());
    }

    #[tokio::test]
    async fn revoked_credential_is_rejected_with_revocation_named() {
        let (_tmp, url, registry, srv) = server().await;
        let secret = registry.lock().await.pair("dev-box", now_unix()).unwrap();
        assert!(registry.lock().await.revoke("dev-box").unwrap());

        let handle = tokio::spawn(async move { srv.accept_one().await });
        let ack = send_hello(&url, &hello("dev-box", NodeAuth::Credential { secret })).await;
        let (code, cause) = sebas_node_link::rejection_of(&ack).unwrap();
        assert_eq!(code, RejectCode::CredentialRevoked);
        assert!(cause.contains("吊销"), "{cause}");
        let _ = handle.await;
    }

    #[tokio::test]
    async fn version_mismatch_reports_both_versions() {
        let (_tmp, url, registry, srv) = server().await;
        let secret = registry.lock().await.pair("dev-box", now_unix()).unwrap();

        let handle = tokio::spawn(async move { srv.accept_one().await });
        let mut probe = hello("dev-box", NodeAuth::Credential { secret });
        probe.protocol_version = PROTOCOL_VERSION + 3;
        let ack = send_hello(&url, &probe).await;
        let (code, cause) = sebas_node_link::rejection_of(&ack).unwrap();
        assert_eq!(code, RejectCode::ProtocolVersionUnsupported);
        assert!(cause.contains(&PROTOCOL_VERSION.to_string()), "{cause}");
        assert!(cause.contains(&(PROTOCOL_VERSION + 3).to_string()), "{cause}");
        let _ = handle.await;
    }

    #[tokio::test]
    async fn malformed_node_id_is_refused_by_the_shared_rule() {
        let (_tmp, url, _registry, srv) = server().await;
        let handle = tokio::spawn(async move { srv.accept_one().await });
        let ack = send_hello(
            &url,
            &hello("bad id", NodeAuth::JoinToken { token: "whatever".into() }),
        )
        .await;
        let (code, cause) = sebas_node_link::rejection_of(&ack).unwrap();
        assert_eq!(code, RejectCode::MalformedHello);
        assert!(cause.contains("非法字符"), "{cause}");
        let _ = handle.await;
    }

    #[tokio::test]
    async fn unknown_token_is_rejected() {
        let (_tmp, url, _registry, srv) = server().await;
        let handle = tokio::spawn(async move { srv.accept_one().await });
        let ack = send_hello(
            &url,
            &hello("dev-box", NodeAuth::JoinToken { token: "not-issued".into() }),
        )
        .await;
        assert_eq!(
            sebas_node_link::rejection_of(&ack).unwrap().0,
            RejectCode::InvalidJoinToken
        );
        let _ = handle.await;
    }

    #[tokio::test]
    async fn same_id_online_twice_is_refused() {
        let (_tmp, url, registry, srv) = server().await;
        let first = registry
            .lock()
            .await
            .issue_join_token(now_unix(), 600)
            .unwrap();
        let srv = Arc::new(srv);

        // 第一条连接保持在线（客户端持有 ws，不关闭）。
        let s1 = Arc::clone(&srv);
        let h1 = tokio::spawn(async move { s1.accept_one().await });
        let (mut ws1, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws1.send(ClientMessage::Text(
            serde_json::to_string(&hello("dev-box", NodeAuth::JoinToken { token: first }))
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
        let ack: HelloAck = match ws1.next().await {
            Some(Ok(ClientMessage::Text(t))) => serde_json::from_str(&t).unwrap(),
            other => panic!("未收到应答：{other:?}"),
        };
        assert!(matches!(ack.outcome, HelloOutcome::Accepted { .. }));
        assert_eq!(registry.lock().await.node("dev-box").unwrap().status(), NodeStatus::Online);

        // 第二台机器用同一个 id 配对 → 必须被拒。
        let second = registry
            .lock()
            .await
            .issue_join_token(now_unix(), 600)
            .unwrap();
        let s2 = Arc::clone(&srv);
        let h2 = tokio::spawn(async move { s2.accept_one().await });
        let ack2 = send_hello(&url, &hello("dev-box", NodeAuth::JoinToken { token: second })).await;
        assert_eq!(
            sebas_node_link::rejection_of(&ack2).unwrap().0,
            RejectCode::NodeIdConflict
        );
        let _ = h2.await;

        drop(ws1);
        let _ = h1.await;
    }

    #[tokio::test]
    async fn two_nodes_both_persist() {
        // 单写者回归：两次配对都必须留在注册表里，不能互相覆盖。
        let (_tmp, url, registry, srv) = server().await;
        let srv = Arc::new(srv);
        for (id, node) in [("node-a", "node-a"), ("node-b", "node-b")] {
            let token = registry.lock().await.issue_join_token(now_unix(), 600).unwrap();
            let s = Arc::clone(&srv);
            let handle = tokio::spawn(async move { s.accept_one().await });
            let ack = send_hello(&url, &hello(id, NodeAuth::JoinToken { token })).await;
            assert!(matches!(ack.outcome, HelloOutcome::Accepted { .. }), "{node}");
            let _ = handle.await;
        }
        let reg = registry.lock().await;
        assert_eq!(reg.nodes().len(), 2, "两次配对都必须落盘");
        assert!(reg.node("node-a").is_some());
        assert!(reg.node("node-b").is_some());
    }

    #[tokio::test]
    async fn missing_hello_is_malformed_not_a_panic() {
        let (_tmp, url, _registry, srv) = server().await;
        let handle = tokio::spawn(async move { srv.accept_one().await });
        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let _ = ws.close(None).await;
        drop(ws);
        match handle.await.unwrap().unwrap() {
            Handled::Malformed { cause } => assert!(!cause.is_empty(), "{cause}"),
            other => panic!("应报 Malformed，实际 {other:?}"),
        }
    }
}
