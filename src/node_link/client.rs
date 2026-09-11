//! 主控侧的节点连接句柄（add-remote-execution-node 3.5 的控制面一半）。
//!
//! 一条已接入的节点链路在这里被拆成三件事：
//!
//! - **请求 / 应答**：按 `id` 关联，超时如实失败——绝不悬挂（悬挂会让上层无法区分
//!   「节点慢」与「节点已经不在」）。
//! - **事件流**：节点主动上报的 turn 批 / 状态 / 退出 / 回收水位线广播给订阅者。
//!   广播语义（有界 + `Lagged`）而不是阻塞：慢订阅者拖不住链路。
//! - **连接态**：断开时**先把所有待决请求失败掉**，再置为已断开——这样上层拿到的
//!   永远是「有成因的失败」，而不是永远等不到的应答。
//!
//! 注意「拒绝」不是 `Err`：节点如实的拒绝（未知会话、容量满、存储触顶…）是**正常
//! 应答**，带着决定重试与否的码。把它当错误会让调用方丢掉重试语义。

use futures_util::{SinkExt, StreamExt};
use sebas_node_link::{Frame, SessionEvent, SessionOp, SessionResult};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

/// 默认请求超时。选 30s：够覆盖一次工具调用的往返，又不会让上层等成假死。
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 事件广播容量：慢订阅者看到 `Lagged` 而不是把链路拖住。
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// 控制面处理**节点发来的**请求。
///
/// 链路是双向的：控制面绝大多数时候是发起方（spawn/prompt/审批…），但**材料拉取**
/// 由节点在会话创建时发起（见 `execution-node` 的材料契约）。没有处理器时节点收到的
/// 应答是「控制面不处理该请求」——如实拒绝，不是静默丢弃。
#[async_trait::async_trait]
pub trait InboundHandler: Send + Sync {
    /// 处理一个来向请求并给出应答。
    async fn handle(&self, op: SessionOp) -> SessionResult;
}

/// 链路层失败（**不含**节点的拒绝——拒绝是正常应答）。
#[derive(Debug, thiserror::Error)]
pub enum NodeLinkError {
    /// 链路已断开（含成因）。
    #[error("节点链路已断开：{cause}")]
    Disconnected {
        /// 断开成因。
        cause: String,
    },
    /// 请求超时。
    #[error("等待节点应答超时（{timeout:?}）")]
    Timeout {
        /// 等待时长。
        timeout: Duration,
    },
    /// 传输层错误（写失败等）。
    #[error("节点链路错误：{cause}")]
    Transport {
        /// 成因。
        cause: String,
    },
}

/// 一条已接入的节点连接。
pub struct NodeConnection {
    node_id: String,
    /// 来向请求处理器（材料拉取等）。`None` = 控制面不处理来向请求。
    handler: Option<Arc<dyn InboundHandler>>,
    /// 对端在握手里上报的能力清单（agent kinds + 可达性、provider 清单、mode 强制能力）。
    /// 控制面据此只提供**可达**的选项，而不是先列出来再让用户撞失败。
    manifest: sebas_node_link::CapabilityManifest,
    next_id: AtomicU64,
    waiters: Mutex<HashMap<u64, oneshot::Sender<SessionResult>>>,
    /// 出站帧（协议文本与 Ping 应答这类控制帧都走它）。
    outbound: mpsc::UnboundedSender<Message>,
    events: broadcast::Sender<SessionEvent>,
    /// `None` = 在线；`Some(cause)` = 已断开。
    closed: Mutex<Option<String>>,
    /// 读到断开后置位，供 `wait_closed` 使用。
    closed_notify: tokio::sync::Notify,
}

impl NodeConnection {
    /// 以**已拆分**的读写两半接管链路（生产路径：`handle_connection` 先拆分再调用）。
    pub fn adopt_split<S>(
        node_id: String,
        manifest: sebas_node_link::CapabilityManifest,
        handler: Option<Arc<dyn InboundHandler>>,
        read_half: futures_util::stream::SplitStream<WebSocketStream<S>>,
        write_half: futures_util::stream::SplitSink<WebSocketStream<S>, Message>,
    ) -> Arc<Self>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (outbound, mut outbound_rx) = mpsc::unbounded_channel::<Message>();
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let conn = Arc::new(Self {
            node_id,
            manifest,
            handler,
            next_id: AtomicU64::new(1),
            waiters: Mutex::new(HashMap::new()),
            outbound,
            events,
            closed: Mutex::new(None),
            closed_notify: tokio::sync::Notify::new(),
        });

        // 写任务。
        let mut sink = write_half;
        tokio::spawn(async move {
            while let Some(message) = outbound_rx.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        // 读循环。
        let reader = Arc::clone(&conn);
        tokio::spawn(async move {
            let mut stream = read_half;
            let cause = loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<Frame>(&text) {
                        Ok(Frame::Response { id, result }) => {
                            if let Some(waiter) = reader.waiters.lock().await.remove(&id) {
                                let _ = waiter.send(result);
                            } else {
                                // 迟到应答（请求已超时）：如实记录，不惊动上层。
                                eprintln!(
                                    "node-link: 节点 {} 的应答 id={id} 无等待者（可能已超时）",
                                    reader.node_id
                                );
                            }
                        }
                        Ok(Frame::Event { event }) => {
                            // 广播失败（无订阅者）不是错误。
                            let _ = reader.events.send(event);
                        }
                        Ok(Frame::Request { id, op }) => {
                            // 节点**可以**向控制面发请求（材料拉取）。没有处理器时
                            // 明确回一个可判别拒绝，而不是静默丢弃让节点等到超时。
                            let result = match &reader.handler {
                                Some(handler) => handler.handle(op).await,
                                None => SessionResult::Rejected {
                                    code: sebas_node_link::SessionRejectCode::NodeError,
                                    cause: "控制面未配置来向请求处理器".into(),
                                },
                            };
                            let response = Frame::Response { id, result };
                            match serde_json::to_string(&response) {
                                Ok(json) => {
                                    let _ = reader.outbound.send(Message::Text(json.into()));
                                }
                                Err(e) => eprintln!(
                                    "node-link: 应答无法序列化（节点 {}）：{e}",
                                    reader.node_id
                                ),
                            }
                        }
                        Err(e) => {
                            eprintln!("node-link: 节点 {} 的帧无法解析：{e}", reader.node_id);
                        }
                    },
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = reader.outbound.send(Message::Pong(payload));
                    }
                    Some(Ok(Message::Close(_))) => break "节点主动关闭".to_string(),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break format!("链路错误：{e}"),
                    None => break "链路被对端关闭".to_string(),
                }
            };
            reader.mark_closed(cause).await;
        });

        conn
    }

    /// 节点标识。
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// 对端上报的能力清单。
    pub fn manifest(&self) -> &sebas_node_link::CapabilityManifest {
        &self.manifest
    }

    /// 是否仍在线。
    pub async fn is_online(&self) -> bool {
        self.closed.lock().await.is_none()
    }

    /// 断开成因（在线时为 `None`）。
    pub async fn close_reason(&self) -> Option<String> {
        self.closed.lock().await.clone()
    }

    /// 等待链路断开（供接入循环在节点离开时收尾）。
    pub async fn wait_closed(&self) {
        while self.closed.lock().await.is_none() {
            self.closed_notify.notified().await;
        }
    }

    /// 订阅节点事件流。
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// 发一次会话操作，用默认超时。
    pub async fn request(&self, op: SessionOp) -> Result<SessionResult, NodeLinkError> {
        self.request_with_timeout(op, DEFAULT_REQUEST_TIMEOUT).await
    }

    /// 发一次会话操作，指定超时。
    ///
    /// 超时**不等于**节点没做——它可能只是应答慢/丢了。调用方要按操作语义判断能否重发
    /// （例如 `Prompt` 重发可能造成重复输入，因此上层应先用 `LogFrom`/`Snapshot` 对账）。
    pub async fn request_with_timeout(
        &self,
        op: SessionOp,
        timeout: Duration,
    ) -> Result<SessionResult, NodeLinkError> {
        // 断开态：立刻如实失败，不写进黑洞。
        if let Some(cause) = self.close_reason().await {
            return Err(NodeLinkError::Disconnected { cause });
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().await.insert(id, tx);

        let frame = Frame::Request { id, op };
        let text = serde_json::to_string(&frame)
            .map_err(|e| NodeLinkError::Transport { cause: format!("无法序列化请求：{e}") })?;
        if self.outbound.send(Message::Text(text.into())).is_err() {
            self.waiters.lock().await.remove(&id);
            let cause = self
                .close_reason()
                .await
                .unwrap_or_else(|| "写通道已关闭".into());
            return Err(NodeLinkError::Disconnected { cause });
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => {
                self.waiters.lock().await.remove(&id);
                let cause = self
                    .close_reason()
                    .await
                    .unwrap_or_else(|| "应答通道被丢弃".into());
                Err(NodeLinkError::Disconnected { cause })
            }
            Err(_) => {
                // 超时后把等待者摘掉：迟到的应答会被读循环如实记录并丢弃。
                self.waiters.lock().await.remove(&id);
                Err(NodeLinkError::Timeout { timeout })
            }
        }
    }

    /// 标记断开并失败所有待决请求（读循环退出时调用）。
    async fn mark_closed(&self, cause: String) {
        {
            let mut closed = self.closed.lock().await;
            if closed.is_none() {
                *closed = Some(cause);
            }
        }
        let mut waiters = self.waiters.lock().await;
        waiters.clear(); // 丢弃 sender → 等待者收到 Err → 翻成 Disconnected
        drop(waiters);
        self.closed_notify.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_link::server::NodeLinkServer;
    use futures_util::{SinkExt, StreamExt};
    use sebas_node_link::{
        CapabilityManifest, Hello, HelloOutcome, NodeAuth, PROTOCOL_VERSION,
    };
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    /// 起服务端 + 一个假节点接入；返回服务端里那条连接句柄。
    async fn connected() -> (
        tempfile::TempDir,
        Arc<NodeLinkServer>,
        Arc<NodeConnection>,
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let server = Arc::new(
            NodeLinkServer::bind("127.0.0.1:0", dir.path().join("nodes.json"))
                .await
                .unwrap(),
        );
        let secret = server
            .registry()
            .lock()
            .await
            .pair("dev-box", crate::node_link::server::now_unix())
            .unwrap();
        let url = format!("ws://{}", server.local_addr().unwrap());

        let srv = Arc::clone(&server);
        tokio::spawn(async move {
            let _ = srv.accept_one().await;
        });

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            node_id: "dev-box".into(),
            auth: NodeAuth::Credential { secret },
            manifest: CapabilityManifest::default(),
        };
        ws.send(ClientMessage::Text(
            serde_json::to_string(&hello).unwrap().into(),
        ))
        .await
        .unwrap();
        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => {
                let ack: sebas_node_link::HelloAck = serde_json::from_str(&t).unwrap();
                assert!(matches!(ack.outcome, HelloOutcome::Accepted { .. }));
            }
            other => panic!("未收到握手应答：{other:?}"),
        }

        // 等连接登记进 live 表。
        let conn = loop {
            if let Some(c) = server.live_connection("dev-box").await {
                break c;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        (dir, server, conn, ws)
    }

    /// 假节点读一帧请求。
    async fn read_request(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> (u64, SessionOp) {
        loop {
            match ws.next().await {
                Some(Ok(ClientMessage::Text(t))) => {
                    if let Frame::Request { id, op } = serde_json::from_str::<Frame>(&t).unwrap() {
                        return (id, op);
                    }
                }
                Some(Ok(_)) => continue,
                other => panic!("未收到请求：{other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn request_round_trips_by_id() {
        let (_d, _server, conn, mut ws) = connected().await;

        let driver = tokio::spawn(async move { conn.request(SessionOp::Ping).await });
        let (id, op) = read_request(&mut ws).await;
        assert!(matches!(op, SessionOp::Ping));
        ws.send(ClientMessage::Text(
            serde_json::to_string(&Frame::Response {
                id,
                result: SessionResult::Pong,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

        assert_eq!(driver.await.unwrap().unwrap(), SessionResult::Pong);
    }

    #[tokio::test]
    async fn two_inflight_requests_do_not_cross() {
        let (_d, _server, conn, mut ws) = connected().await;

        let c1 = Arc::clone(&conn);
        let h1 = tokio::spawn(async move {
            c1.request(SessionOp::Snapshot {
                session_id: "s-1".into(),
            })
            .await
        });
        let c2 = Arc::clone(&conn);
        let h2 = tokio::spawn(async move { c2.request(SessionOp::ListSessions).await });

        let (id_a, op_a) = read_request(&mut ws).await;
        let (id_b, op_b) = read_request(&mut ws).await;
        assert_ne!(id_a, id_b);

        // **故意乱序**应答：后到的请求先回。按 id 关联意味着两条都不会串。
        let (first, second) = if matches!(op_a, SessionOp::ListSessions) {
            ((id_a, SessionResult::Sessions { sessions: vec![] }), (id_b, SessionResult::Pong))
        } else {
            (
                (id_b, SessionResult::Sessions { sessions: vec![] }),
                (id_a, SessionResult::Pong),
            )
        };
        for (id, result) in [second, first] {
            ws.send(ClientMessage::Text(
                serde_json::to_string(&Frame::Response { id, result })
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        }

        let r1 = h1.await.unwrap().unwrap();
        let r2 = h2.await.unwrap().unwrap();
        assert!(matches!(r1, SessionResult::Pong));
        assert!(matches!(r2, SessionResult::Sessions { .. }));
        let _ = (op_b, op_a);
    }

    #[tokio::test]
    async fn rejection_is_a_normal_result_not_an_error() {
        let (_d, _server, conn, mut ws) = connected().await;

        let c = Arc::clone(&conn);
        let h = tokio::spawn(async move {
            c.request(SessionOp::Prompt {
                session_id: "ghost".into(),
                text: "x".into(),
            })
            .await
        });
        let (id, _op) = read_request(&mut ws).await;
        ws.send(ClientMessage::Text(
            serde_json::to_string(&Frame::Response {
                id,
                result: SessionResult::Rejected {
                    code: sebas_node_link::SessionRejectCode::UnknownSession,
                    cause: "会话 ghost 不存在".into(),
                },
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

        let result = h.await.unwrap().unwrap();
        let (code, cause) = sebas_node_link::session_rejection_of(&result).unwrap();
        assert_eq!(code, sebas_node_link::SessionRejectCode::UnknownSession);
        assert!(cause.contains("ghost"));
    }

    #[tokio::test]
    async fn events_reach_subscribers() {
        let (_d, _server, conn, mut ws) = connected().await;
        let mut events = conn.subscribe();

        ws.send(ClientMessage::Text(
            serde_json::to_string(&Frame::Event {
                event: SessionEvent::TurnBatch {
                    session_id: "s-1".into(),
                    epoch: 1,
                    from_seq: 1,
                    entries: vec![],
                    coalesced_overflow: false,
                },
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(SessionEvent::TurnBatch { session_id, .. })) => assert_eq!(session_id, "s-1"),
            other => panic!("未收到事件：{other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_is_reported_and_late_reply_is_dropped() {
        let (_d, _server, conn, mut ws) = connected().await;

        let c = Arc::clone(&conn);
        let h = tokio::spawn(async move {
            c.request_with_timeout(SessionOp::Ping, Duration::from_millis(80))
                .await
        });
        let (id, _op) = read_request(&mut ws).await;

        match h.await.unwrap() {
            Err(NodeLinkError::Timeout { .. }) => {}
            other => panic!("应超时，实际 {other:?}"),
        }

        // 迟到的应答不应把链路搞坏：后续请求照常工作。
        ws.send(ClientMessage::Text(
            serde_json::to_string(&Frame::Response {
                id,
                result: SessionResult::Pong,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

        let c2 = Arc::clone(&conn);
        let h2 = tokio::spawn(async move { c2.request(SessionOp::Ping).await });
        let (id2, _op2) = read_request(&mut ws).await;
        ws.send(ClientMessage::Text(
            serde_json::to_string(&Frame::Response {
                id: id2,
                result: SessionResult::Pong,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        assert_eq!(h2.await.unwrap().unwrap(), SessionResult::Pong);
    }

    #[tokio::test]
    async fn disconnect_fails_pending_requests_instead_of_hanging() {
        let (_d, _server, conn, ws) = connected().await;

        let c = Arc::clone(&conn);
        let h = tokio::spawn(async move { c.request_with_timeout(SessionOp::Ping, Duration::from_secs(60)).await });

        // 让请求先上路，再断开节点。
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(ws);

        match tokio::time::timeout(Duration::from_secs(10), h).await {
            Ok(Ok(Err(NodeLinkError::Disconnected { cause }))) => {
                assert!(!cause.is_empty(), "断开要给成因");
            }
            other => panic!("断开应让待决请求如实失败，实际 {other:?}"),
        }
        assert!(conn.close_reason().await.is_some());
        // 断开后新请求立刻失败，不写进黑洞。
        assert!(matches!(
            conn.request(SessionOp::Ping).await,
            Err(NodeLinkError::Disconnected { .. })
        ));
    }
}
