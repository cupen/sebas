//! `sebas-node` 二进制入口。
//!
//! 两段式启动：同步解析段（[`startup`]）→ 异步长驻段（[`LinkClient::run`]）。
//! 任何一段的**永久性**失败都走统一的启动失败出口（stderr 末行
//! `startup-failure: <原因>` + `EX_TEMPFAIL` 75），与主控 `sebas` 共用
//! `sebas-startup` 里的同一份契约实现。

use clap::Parser;
use sebas_node::Cli;
use sebas_node::body::NodeBodyFactory;
use sebas_node::identity::IdentityStore;
use sebas_node::link::LinkClient;
use sebas_node::startup;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let started = match startup(&cli) {
        Ok(started) => started,
        Err(e) => sebas_startup::exit_startup_failure(&e.to_string()),
    };

    if started.checked_only {
        // 自检：把解析结果打给人看，正常退出（0）。
        println!("{}", started.check_report());
        return;
    }

    let store = IdentityStore::new(started.state_dir.clone());
    // 执行体工厂：echo 之外还有配置里真正接入了运行时的 agent kind（6.3）。
    // 链路在握手成功后把控制面告知的 router 地址填进 `router` 槽（7.2）。
    let factory = std::sync::Arc::new(NodeBodyFactory::new(started.body.clone()));
    let router = factory.router_cell();
    let client = LinkClient::new(
        started.control_plane.clone(),
        started.node_id.clone(),
        store,
        cli.join_token.clone(),
        // 会话日志落在节点状态目录下：节点重启后仍能被主控拉回（4.1）。
        started.state_dir.join("sessions"),
        // 操作者级材料落在这里（按版本目录隔离）。
        started.state_dir.join("materials"),
        started.max_sessions as usize,
    )
    .with_manifest(started.manifest.clone())
    // 保留期是**节点自己的**策略（4.4 / r2），从节点配置灌进链路层。
    .with_retention_days(started.log_retention_days)
    .with_body_factory(factory, router)
    .await;

    // 瞬时失败在 run() 内部退避重试；返回 Err 即"永久不可用"
    // （凭据被吊销 / 协议版本不兼容 / wss 尚未支持 / 身份未配对），
    // 此时如实退出让监督者看到成因，而不是静默空转。
    if let Err(e) = std::sync::Arc::new(client).run().await {
        sebas_startup::exit_startup_failure(&e.to_string());
    }
}
