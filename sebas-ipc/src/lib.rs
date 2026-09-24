//! 跨平台本地 IPC 传输层 + 跨进程协议之家
//! （核心会话通道 / 控制 RPC / router 状态订阅共用）。
//!
//! 基于 `interprocess` 的 local socket 抽象：Unix 上是 Unix domain socket
//! （文件路径语义与历史行为一致，含僵尸 socket 回收），Windows 上由
//! `GenericFilePath` 确定性映射为 named pipe。配置里的 socket 路径跨平台
//! 不变，服务端与客户端从同一路径得到同一端点。
//!
//! - [`bind`] / [`accept`]：服务端；
//! - [`connect`]：客户端（Windows 上所有管道实例忙时做有界重试）；
//! - [`split`]：拆成读写两半（[`ReadHalf`] / [`WriteHalf`]）；
//! - [`protocol`]：各边界的 wire 类型与握手 / framing 助手；
//! - [`secret`]：通道 secret 的共享发现实现（env → secret 文件）。
//!
//! 安全基线：应用层 secret 握手由各通道协议自带。Unix 上 socket 文件
//! 0600 与 SO_PEERCRED uid 校验由调用方负责（socket 文件路径仍可由调用方
//! 直接操作）；Windows named pipe 依赖默认 ACL（仅创建者与
//! SYSTEM/Administrators 可访问）+ secret。
//!
//! # 准入面（unify-ipc-protocol-home 1.3）
//!
//! 本 crate 是**协议之家**：跨进程协议的定义只允许有一份，谁说话谁依赖它。
//! 一个类型 / 函数进入本 crate 必须属于下面三类之一：
//!
//! 1. **transport**——端点绑定、连接、接受、拆半（[`bind`] / [`connect`] /
//!    [`accept`] / [`split`]）；
//! 2. **wire 类型**——跨进程边界上过线的消息、帧、载荷（[`protocol`]），
//!    含它们各自的 serde 形状与固有助手；
//! 3. **握手 / framing 助手**——握手（secret + 版本协商，[`protocol`]）与
//!    NDJSON 一行一帧的分帧（[`protocol::encode_line`] /
//!    [`protocol::decode_line`]），以及 secret 的**发现**（[`secret`]）。
//!
//! ## 不得含角色实现
//!
//! **不允许**进来的东西（纪律，不是建议）：
//!
//! - 任何**角色实现**的关注点：core / webui / router / im 的运行时类型、
//!   trait、句柄，以及执行节点 `sebas-node` 的任何类型。角色要说话就**依赖**
//!   本 crate，不得反过来被本 crate 依赖——否则协议 crate 一旦引用角色类型
//!   即成依赖环（`dispatch → router`、`webui → router` 两条边已存在，见
//!   design D3）。
//! - 任何**域表名**（持久层概念）。本 crate 不认识任何数据库、表或 schema。
//! - **secret 文件的生命周期**（生成 / 原子落盘 / 权限）：那是 core 进程的
//!   职责，留在根 crate 的 `core_channel::secret`。这里只做**解析**。
//!
//! 两条准入面各有一条机械断言守着（根 crate
//! `tests/ipc_protocol_home_test.rs`）：依赖图里不得出现角色实现，公开源码里
//! 不得出现角色 crate 引用或域表名字面量。违反即测试红。
//!
//! ## 演进纪律（前向兼容三规则）
//!
//! 新增字段**必须**带 serde 默认值；取 wire 值的枚举**必须**保留未知值路径
//! （`#[serde(other)] Unknown`）；删字段 / 改字段名 / 改取值属**破坏性变更**，
//! 必须在 change 里显式声明并同步更新 golden fixture（见 [`protocol`] 模块
//! 文档与 `openspec/changes/unify-ipc-protocol-home/specs/ipc-protocol-home/spec.md`）。

use std::io;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::tokio::Stream as TokioStream;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};

pub mod protocol;
pub mod secret;

pub type IpcStream = interprocess::local_socket::tokio::Stream;
pub type IpcListener = interprocess::local_socket::tokio::Listener;
pub type ReadHalf = interprocess::local_socket::tokio::RecvHalf;
pub type WriteHalf = interprocess::local_socket::tokio::SendHalf;

/// ERROR_PIPE_BUSY：Windows named pipe 的所有实例都忙（服务端尚未换发下
/// 一个实例）。客户端对它做有界重试，而不是把「服务端正在接下一个连接」
/// 误报为不可达。
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 232;

const CONNECT_BUSY_RETRY: Duration = Duration::from_secs(5);

/// 把配置里的 socket 路径映射为平台端点名：
/// - Unix：路径本身（UDS 文件，与历史行为一致）；
/// - Windows：确定性映射为 `\\.\pipe\sebas\<路径>`（分隔符统一为 `/`），
///   服务端与客户端从同一路径得到同一管道。注意 Windows 管道全名上限
///   256 字符，超长的临时目录路径可能放不下。
fn fs_name(path: &Path) -> io::Result<interprocess::local_socket::Name<'static>> {
    #[cfg(unix)]
    return path.to_path_buf().to_fs_name::<GenericFilePath>();
    #[cfg(windows)]
    {
        let cleaned = path.to_string_lossy().replace('\\', "/");
        let cleaned = cleaned.trim_start_matches('/');
        PathBuf::from(format!(r"\\.\pipe\sebas/{cleaned}"))
            .to_fs_name::<GenericFilePath>()
    }
    .map_err(|e| io::Error::other(format!("invalid socket path {}: {e}", path.display())))
}

/// 绑定监听端。Unix 上 UDS 文件路径由 interprocess 负责绑定（僵尸 socket
/// 回收默认开启）；Windows 上映射为 named pipe。活实例互斥：Unix 上 bind
/// 到仍应答的路径会失败（调用方在此前会先探测并给出更准确的错误信息），
/// Windows 上由 named pipe 实例语义决定。
pub fn bind(path: &Path) -> io::Result<IpcListener> {
    let name = fs_name(path)?;
    ListenerOptions::new().name(name).create_tokio()
}

/// 客户端连接。
pub async fn connect(path: &Path) -> io::Result<IpcStream> {
    let deadline = Instant::now() + CONNECT_BUSY_RETRY;
    loop {
        let name = fs_name(path)?;
        match <IpcStream as TokioStream>::connect(name).await {
            Ok(stream) => return Ok(stream),
            Err(e) if is_busy(&e) && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

fn is_busy(e: &io::Error) -> bool {
    #[cfg(windows)]
    if e.raw_os_error() == Some(ERROR_PIPE_BUSY) {
        return true;
    }
    e.kind() == io::ErrorKind::WouldBlock
}

/// 接受一个连接（转发 interprocess 的 `Listener` trait，调用方无需自行导入）。
pub async fn accept(listener: &IpcListener) -> io::Result<IpcStream> {
    use interprocess::local_socket::traits::tokio::Listener as _;
    listener.accept().await
}

/// 拆成（读半, 写半）。与 tokio 的 `into_split` 顺序一致。
pub fn split(stream: IpcStream) -> (ReadHalf, WriteHalf) {
    stream.split()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn bind_connect_accept_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sebas-ipc-test.sock");
        let listener = bind(&path).expect("bind");
        let mut client = connect(&path).await.expect("connect");
        let mut server = accept(&listener).await.expect("accept");

        client.write_all(b"ping").await.unwrap();
        client.flush().await.unwrap();
        let mut buf = [0u8; 4];
        server.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");

        server.write_all(b"pong").await.unwrap();
        server.flush().await.unwrap();
        let mut buf2 = [0u8; 4];
        client.read_exact(&mut buf2).await.unwrap();
        assert_eq!(&buf2, b"pong");
    }

    // 5.1 验收（服务端侧）：活监听必须拒绝二次 bind。Unix 上 UDS 文件
    // 仍应答 → AddrInUse；Windows 上 named pipe 单实例由内核保证。
    #[tokio::test]
    async fn second_bind_on_live_listener_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dup.sock");
        let _l1 = bind(&path).expect("first bind");
        assert!(bind(&path).is_err(), "live socket must refuse rebind");
    }
}
