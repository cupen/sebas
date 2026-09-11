//! 节点链路的**装配**：打开注册表、绑定监听、必要时签发 bootstrap 配对 token。
//!
//! 设计 D13：入站端点由 **core** 托管。装配在 core **达到 ready 之前**完成：bind 失败
//! 就是启动失败（与 core session channel 同款处置），进程以 75 退出，而不是顶着
//! "监听没起来"继续对外服务。
//!
//! 拆成三步而不是一个整体，是为了让调用方能定下**顺序**：
//!
//! 1. [`open_registry`]：先拿到写者句柄——通道也要用它（2.7 的管理入口经通道暴露）；
//! 2. [`serve_registry`]：绑定并开始服务；
//! 3. [`issue_bootstrap_token`]：**两端都 bind 成功之后**才签发首配 token——否则
//!    通道 bind 失败时操作者会先看到日志里的 token、却因启动失败而作废。
//!
//! 单写者：本模块返回的句柄就是监听器自己那一份。任何签发 token / 吊销节点 /
//! 查看在线态的上层都**必须**用它，绝不可另开 `NodeRegistry` 写同一个文件。

use crate::config::NodeLinkConfig;
use crate::node_link::registry::{NodeRegistry, RegistryError};
use crate::node_link::server::{NodeLinkServer, now_unix};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, watch};

/// 已开始服务的节点链路。
pub struct ServedNodeLink {
    /// 实际绑定的监听地址（端口为 0 时这里是内核分配的真实端口）。
    pub listen: String,
    /// 控制面材料仓（节点按需拉取；未配置时如实拒绝节点）。
    pub materials: std::sync::Arc<crate::node_link::MaterialStore>,
    /// 与监听共享的注册表写者句柄。
    pub registry: Arc<Mutex<NodeRegistry>>,
    /// 关闭信号：置 true 即停止接受新连接。
    pub shutdown: watch::Sender<bool>,
}

impl ServedNodeLink {
    /// 请求优雅关闭（幂等）。
    pub fn close(&self) {
        let _ = self.shutdown.send(true);
    }
}

/// 一次完整装配的结果（`open` + `serve` + `issue` 的组合形态，供测试与简单调用方使用）。
pub struct ArmedNodeLink {
    /// 已开始的服务。
    pub served: ServedNodeLink,
    /// 本次启动是否签发了 bootstrap 配对 token；有值则**只出现这一次**，
    /// 调用方负责把它告诉操作者（core 记进日志）。
    pub bootstrap_token: Option<String>,
}

/// 第 1 步：打开注册表（写者句柄）。
pub fn open_registry(
    cfg: &NodeLinkConfig,
    config_path: &Path,
) -> Result<Arc<Mutex<NodeRegistry>>, RegistryError> {
    let registry = NodeRegistry::open(cfg.registry_path(config_path))?;
    Ok(Arc::new(Mutex::new(registry)))
}

/// 第 2 步：绑定并开始服务。
pub async fn serve_registry(
    cfg: &NodeLinkConfig,
    registry: Arc<Mutex<NodeRegistry>>,
    // 连接生命周期观察者（远端会话投影 5.1/5.3）；`None` = 不上报接入/断开。
    observer: Option<Arc<dyn crate::node_link::server::ConnectionObserver>>,
    // 主控的 router 端点（7.2）：告知节点后它才能把模型流量指回主控。
    router: crate::node_link::server::RouterEndpoint,
) -> Result<ServedNodeLink, std::io::Error> {
    // 材料仓：生产路径默认挂上（未配置内容时对节点如实拒绝，见 MaterialStore）。
    let materials = crate::node_link::MaterialStore::new();
    let mut server = NodeLinkServer::bind_with(&cfg.listen, Arc::clone(&registry))
        .await?
        .with_inbound_handler(Arc::clone(&materials) as Arc<dyn crate::node_link::client::InboundHandler>);
    if let Some(observer) = observer {
        server = server.with_observer(observer);
    }
    server = server.with_router_endpoint(router);
    let listen = server
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| cfg.listen.clone());
    let (shutdown, rx) = watch::channel(false);
    tokio::spawn(async move {
        if let Err(e) = server.serve(rx).await {
            eprintln!("node-link: 服务端退出：{e}");
        }
    });
    Ok(ServedNodeLink {
        listen,
        materials,
        registry,
        shutdown,
    })
}

/// 第 3 步：仅当「注册表里既没有节点、也没有待用 token」时签发一次首配 token。
///
/// 它是把**第一台**节点接进来的入口；一旦有节点或已有待用 token，就不再多发
/// （否则每次重启都往日志里吐一个可用的配对凭据）。
pub async fn issue_bootstrap_token(
    registry: &Mutex<NodeRegistry>,
    ttl_secs: u64,
) -> Result<Option<String>, RegistryError> {
    let mut reg = registry.lock().await;
    let now = now_unix();
    if reg.nodes().is_empty() && reg.pending_join_tokens(now).is_empty() {
        Ok(Some(reg.issue_join_token(now, ttl_secs as i64)?))
    } else {
        Ok(None)
    }
}

/// 组合形态：打开 → 服务 → 签发首配 token。
pub async fn arm(
    cfg: &NodeLinkConfig,
    config_path: &Path,
) -> Result<ArmedNodeLink, std::io::Error> {
    let registry = open_registry(cfg, config_path).map_err(std::io::Error::other)?;
    let served = serve_registry(
        cfg,
        Arc::clone(&registry),
        None,
        crate::node_link::server::RouterEndpoint::default(),
    )
    .await?;
    let bootstrap_token = issue_bootstrap_token(&registry, cfg.bootstrap_token_ttl_secs)
        .await
        .map_err(std::io::Error::other)?;
    Ok(ArmedNodeLink {
        served,
        bootstrap_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use sebas_node_link::{CapabilityManifest, Hello, HelloOutcome, NodeAuth, PROTOCOL_VERSION};
    use tokio_tungstenite::tungstenite::Message;

    fn config(dir: &Path) -> NodeLinkConfig {
        NodeLinkConfig {
            enabled: true,
            // 端口 0：内核分配，测试不抢固定端口。
            listen: "127.0.0.1:0".into(),
            registry_file: Some(dir.join("nodes.json").to_string_lossy().into_owned()),
            bootstrap_token_ttl_secs: 600,
        }
    }

    async fn pair(url: &str, node_id: &str, token: &str) -> sebas_node_link::HelloAck {
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            node_id: node_id.into(),
            auth: NodeAuth::JoinToken {
                token: token.into(),
            },
            manifest: CapabilityManifest::default(),
        };
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        ws.send(Message::Text(
            serde_json::to_string(&hello).unwrap().into(),
        ))
        .await
        .unwrap();
        let text = match ws.next().await {
            Some(Ok(Message::Text(t))) => t,
            other => panic!("未收到应答：{other:?}"),
        };
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn arm_binds_and_issues_a_bootstrap_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let armed = arm(&cfg, &dir.path().join("config.toml")).await.unwrap();
        let token = armed
            .bootstrap_token
            .clone()
            .expect("首次装配应签发 bootstrap token");
        assert_eq!(token.len(), 64);
        assert!(armed.served.listen.starts_with("127.0.0.1:"), "{}", armed.served.listen);

        // 用这个 token 真跑一次配对：节点应被登记且被判接受。
        let ack = pair(
            &format!("ws://{}", armed.served.listen),
            "dev-box",
            &token,
        )
        .await;
        match ack.outcome {
            HelloOutcome::Accepted { credential } => {
                assert!(credential.is_some(), "配对应答必须交付长期凭据");
            }
            other => panic!("应被接受，实际 {other:?}"),
        }
        assert!(armed.served.registry.lock().await.node("dev-box").is_some());
        armed.served.close();
    }

    #[tokio::test]
    async fn arm_does_not_reissue_a_bootstrap_token_when_one_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let first = arm(&cfg, &dir.path().join("config.toml")).await.unwrap();
        assert!(first.bootstrap_token.is_some());
        first.served.close();

        // 同一个注册表再装配一次：仍有待用 token → 不再吐新的。
        let second = arm(&cfg, &dir.path().join("config.toml")).await.unwrap();
        assert!(
            second.bootstrap_token.is_none(),
            "已有待用 token 时不应再签发，否则每次重启都往日志里吐凭据"
        );
        second.served.close();
    }

    #[tokio::test]
    async fn arm_does_not_reissue_after_a_node_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let first = arm(&cfg, &dir.path().join("config.toml")).await.unwrap();
        let token = first.bootstrap_token.clone().unwrap();
        let _ = pair(
            &format!("ws://{}", first.served.listen),
            "dev-box",
            &token,
        )
        .await;
        first.served.close();

        let second = arm(&cfg, &dir.path().join("config.toml")).await.unwrap();
        assert!(second.bootstrap_token.is_none(), "已有节点时不应再签发");
        assert!(second.served.registry.lock().await.node("dev-box").is_some());
        second.served.close();
    }

    #[tokio::test]
    async fn close_stops_accepting_new_connections() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let armed = arm(&cfg, &dir.path().join("config.toml")).await.unwrap();
        let url = format!("ws://{}", armed.served.listen);
        armed.served.close();
        // 关闭后新连接应在短时间内失败（accept 循环已退出，监听套接字释放）。
        let mut refused = false;
        for _ in 0..50 {
            if tokio_tungstenite::connect_async(&url).await.is_err() {
                refused = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(refused, "关闭后不应再接受连接");
    }

    #[tokio::test]
    async fn steps_can_be_ordered_by_the_caller() {
        // 调用方自己排顺序：先开注册表 → 发（或先不发）首配 token。
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let registry = open_registry(&cfg, &dir.path().join("config.toml")).unwrap();
        // bind 之前签发：句柄可用，但服务还没起（这正是 core 里要避免的顺序，
        // 这里只证明两步确实可以分开）。
        let token = issue_bootstrap_token(&registry, 60).await.unwrap();
        assert!(token.is_some());
        let served = serve_registry(
            &cfg,
            Arc::clone(&registry),
            None,
            crate::node_link::server::RouterEndpoint::default(),
        )
        .await
        .unwrap();
        // 再次签发：已有待用 token → 不再签发。
        assert!(issue_bootstrap_token(&registry, 60).await.unwrap().is_none());
        served.close();
    }
}
