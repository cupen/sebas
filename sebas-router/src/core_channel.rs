//! 核心通道状态订阅客户端 (add-state-store 5.3)。
//!
//! 当 core channel 本地 socket 可用时, 订阅状态变更通知, 收到通知后触发
//! provider/alias 热重载。通道不可用时降级为文件监听 (hot_reload 保持)。
//!
//! 协议: NDJSON over 本地 IPC (Unix socket / Windows named pipe)。帧类型、
//! 握手（含版本协商）与请求构造**全部复用 `sebas_ipc::protocol` 的共享定义**
//! (unify-ipc-protocol-home 4.1)——本文件不再重声明任何帧子集、不再手搓
//! `json!` 请求: core 侧改一个字段名, 这里直接编译失败。
//!
//! 握手 secret 经 `sebas_ipc::secret::ChannelSecret` 的共享发现实现解析
//! (env → secret 文件, 4.2)。订阅是持久流——先一帧全域快照, 之后每帧一条
//! scope 变更(一串提交已由服务端合并)。断连时保持最后有效配置, 由
//! `ReloadStatus` 记录不可用状态供 `/admin/stats` 暴露。

use std::path::{Path, PathBuf};
use std::time::Duration;

use sebas_ipc::protocol::{
    ChannelHandshake, ChannelHandshakeAck, CoreChannelRequest, CoreChannelResponse,
    StateStreamFrame, WireFrame,
};
use sebas_ipc::secret::ChannelSecret;
use crate::server::AppState;

/// 握手 secret 解析（harden-core-channel-deployment 2.2，design D2）。
///
/// （unify-ipc-protocol-home 4.2）此前本文件自己复刻了一份「env → secret 文件」
/// 的解析；现在只保留**落点推导**（router 子进程由 watchdog 注入
/// `SEBAS_ROUTER_CONFIG` 为其 `--config` 同值，故 secret 文件在它所在目录下，
/// 与 core 自动武装落盘位置一致），解析本身走共享实现
/// [`sebas_ipc::secret::ChannelSecret`]。每次连接尝试重新 `current()`：
/// core 重启换钥后重连天然自愈。
fn channel_secret() -> String {
    let file = std::env::var("SEBAS_ROUTER_CONFIG").ok().map(|cfg_path| {
        let cfg = Path::new(&cfg_path);
        let dir = cfg
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        dir.join(sebas_ipc::secret::SECRET_FILE_NAME)
    });
    ChannelSecret::from_env_or_file(file).current()
}

/// 核心通道 socket 路径, 由 `SEBAS_CORE_SOCKET` 环境变量指定。
/// 未设置时返回 `None` (通道不可用, 走文件监听)。
pub(crate) fn socket_path() -> Option<PathBuf> {
    let raw = std::env::var("SEBAS_CORE_SOCKET").ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(raw))
}

/// 启动核心通道订阅循环 (tokio task)。
/// 当 socket 路径可用时, 连接并订阅状态变更; 不可用时静默返回。
pub fn spawn_subscriber(state: AppState) {
    let Some(path) = socket_path() else {
        tracing::info!("core channel socket not set (SEBAS_CORE_SOCKET), using file watch");
        return;
    };
    tokio::spawn(async move {
        subscribe_loop(state, path).await;
    });
}

/// 订阅循环: 连接 → 握手 → 发送 StateSubscribe → 接收通知 → 触发 reload。
/// 断连时退避重连, 并记录「数据源不可用」供 /admin/stats。
async fn subscribe_loop(state: AppState, path: PathBuf) {
    let mut backoff = Duration::from_secs(1);
    // core 停用时 socket 永远不会出现——这是预期状态，只在 debug 级别记录
    // （运行状态可从 /admin/stats 的 reload_status 看到）。只有「曾经连上后
    // 断开」才算通道故障，WARN 一次（每轮连续失败一条）。
    let mut ever_connected = false;
    let mut reported = false;
    loop {
        // 每次尝试前标记数据源不可用 (重连成功后由 subscribe_once 清除)。
        state
            .reload_status
            .record_source_unavailable("core channel 断连, 保持最后有效配置");
        match subscribe_once(&state, &path).await {
            Ok(()) => {
                // 正常断开 (core 关闭通道后重连)。
                backoff = Duration::from_secs(1);
                ever_connected = true;
                reported = false;
            }
            Err(e) => {
                // connect 阶段失败 = 通道从未建立；其余错误说明连接曾建立过。
                if !e.contains("core channel connect failed") {
                    ever_connected = true;
                }
                if !reported {
                    if ever_connected {
                        tracing::warn!(error = %e, "core channel lost, reconnecting, backoff {backoff:?}");
                    } else {
                        tracing::debug!(
                            error = %e,
                            "core channel unavailable (core disabled or starting), hot reload via file watch"
                        );
                    }
                    reported = true;
                } else {
                    tracing::debug!(error = %e, "core channel subscribe failed, backoff {backoff:?}");
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// 订阅触发的 reload（5.3 投影）：优先从 core channel 拉 providers/aliases
/// 快照重建配置；通道快照失败时回退文件 overlay（`reload_and_swap`）。
/// 成功后清除「数据源不可用」，失败保旧内核并记录错误。
pub(crate) async fn reload_from_channel(state: &AppState, path: &Path) {
    let result = async {
        let snapshot = fetch_state_snapshot(path, "providers").await?;
        let mut cfg = crate::admin::rebuild_from_seed(state)?;
        // apply_overlay_value 会覆盖 providers + deleted + model_aliases。
        cfg.apply_overlay_value(&snapshot)
            .map_err(|e| e.to_string())?;
        state.swap_core(cfg).map_err(|e| format!("hot swap failed: {e}"))?;
        Ok::<(), String>(())
    }
    .await;
    match result {
        Ok(()) => {
            state.reload_status.record_source_ok();
            state.reload_status.record_ok_quiet();
            tracing::info!("core channel snapshot applied, config hot swapped");
        }
        Err(e) => {
            tracing::warn!("core channel snapshot failed, keeping old config: {e}");
            state.reload_status.record_err(&e);
            // 快照拉取失败但通道仍活着（如 parse 错误）→ 尝试文件回退。
            if e.contains("core channel connect failed")
                || e.contains("handshake")
                || e.contains("response read failed")
            {
                tracing::info!("core channel unavailable, falling back to file overlay reload");
                let _ = crate::admin::reload_and_swap(state);
            }
        }
    }
}

/// 通用短连接通道请求：握手 → 发一帧请求 → 收一帧响应。
/// 请求/响应均为 NDJSON on 本地 IPC，类型是**共享定义**（core 侧同一份）。
async fn channel_request(
    path: &Path,
    req: &CoreChannelRequest,
) -> Result<CoreChannelResponse, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stream = sebas_ipc::connect(path)
        .await
        .map_err(|e| format!("core channel connect failed: {e}"))?;
    let (reader, mut writer) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(reader);

    // 握手: secret 经共享发现解析，握手帧经共享类型构造（含版本协商）。
    let hs = ChannelHandshake::new(channel_secret())
        .to_line()
        .map_err(|e| format!("handshake serialize failed: {e}"))?;
    writer
        .write_all(hs.as_bytes())
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("handshake flush failed: {e}"))?;

    let mut ack = String::new();
    reader
        .read_line(&mut ack)
        .await
        .map_err(|_| "handshake ack read failed (secret may have been rejected)".to_string())?;
    if ack.trim().is_empty() {
        return Err("handshake ack empty".into());
    }
    // 握手应答是共享类型：不受支持的版本在这里被**指名双方的拒绝**挡下。
    let ack = ChannelHandshakeAck::from_line(ack.trim())
        .map_err(|e| format!("handshake ack parse failed: {e}"))?;
    if let Some(cause) = ack.cause() {
        return Err(format!("handshake rejected: {cause}"));
    }

    let req_line = req
        .to_line()
        .map_err(|e| format!("request serialize failed: {e}"))?;
    writer
        .write_all(req_line.as_bytes())
        .await
        .map_err(|e| format!("request write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("request flush failed: {e}"))?;

    let mut resp = String::new();
    reader
        .read_line(&mut resp)
        .await
        .map_err(|_| "response read failed".to_string())?;
    CoreChannelResponse::from_line(resp.trim()).map_err(|e| format!("response parse failed: {e}"))
}

/// 短连接请求 core 的 `StateSnapshot{domain}`，返回 payload。
/// 订阅触发 reload 时用于从通道拉最新 providers/aliases（5.3 投影）。
pub(crate) async fn fetch_state_snapshot(
    path: &Path,
    domain: &str,
) -> Result<serde_json::Value, String> {
    let resp = channel_request(
        path,
        &CoreChannelRequest::StateSnapshot {
            domain: domain.to_string(),
        },
    )
    .await?;
    match resp {
        CoreChannelResponse::StateSnapshot { domain: got, payload } => {
            if got != domain {
                return Err(format!("unexpected response domain: {got}"));
            }
            Ok(payload)
        }
        other => Err(format!("unexpected response cmd: {other:?}")),
    }
}

/// 一次完整的订阅会话: 连接 → 握手 → 发送请求 → 读快照 → 持续读通知。
/// 返回时连接已断开 (调用方退避重连)。
async fn subscribe_once(state: &AppState, path: &Path) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stream = sebas_ipc::connect(path)
        .await
        .map_err(|e| format!("core channel connect failed: {e}"))?;
    let (reader, mut writer) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(reader);

    // 握手: secret 经共享发现解析；订阅是长连接，重连时重读文件 →
    // core 重启换钥自愈。
    let hs = ChannelHandshake::new(channel_secret())
        .to_line()
        .map_err(|e| format!("handshake serialize failed: {e}"))?;
    writer
        .write_all(hs.as_bytes())
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("handshake flush failed: {e}"))?;

    // 读握手 ack：共享应答类型，版本不受支持时**指名双方版本**地失败。
    let mut ack = String::new();
    reader
        .read_line(&mut ack)
        .await
        .map_err(|_| "handshake ack read failed (secret may have been rejected)".to_string())?;
    if ack.trim().is_empty() {
        return Err("handshake ack empty".into());
    }
    let ack = ChannelHandshakeAck::from_line(ack.trim())
        .map_err(|e| format!("handshake ack parse failed: {e}"))?;
    if let Some(cause) = ack.cause() {
        return Err(format!("handshake rejected: {cause}"));
    }

    // 发送 StateSubscribe 请求（共享请求类型构造，不再手搓 json!）。
    let req_line = CoreChannelRequest::StateSubscribe
        .to_line()
        .map_err(|e| format!("request serialize failed: {e}"))?;
    writer
        .write_all(req_line.as_bytes())
        .await
        .map_err(|e| format!("request write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("request flush failed: {e}"))?;

    // 读帧循环: 先快照, 之后持续读变更通知。帧类型是**共享定义**——
    // core 侧改字段名，这里编译失败（而不是静默不匹配）。
    let mut line = String::new();
    let mut got_snapshot = false;
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("stream read failed: {e}"))?;
        if n == 0 {
            return Err("connection dropped".into());
        }
        let frame: StateStreamFrame =
            StateStreamFrame::from_line(line.trim()).map_err(|e| format!("frame parse failed: {e}"))?;
        match frame {
            StateStreamFrame::Snapshot { .. } => {
                // 订阅成功: 连接健康, 清除数据源不可用并触发一次完全 reload。
                got_snapshot = true;
                state.reload_status.record_source_ok();
                tracing::info!("core channel subscribed, reloading providers");
                reload_from_channel(state, path).await;
            }
            StateStreamFrame::Changed { scope } => {
                if !got_snapshot {
                    // 未见快照先见变更, 协议外但可容错: 依旧触发 reload。
                    tracing::warn!("got change before snapshot (scope={scope})");
                }
                tracing::info!("core channel state change: scope={scope}, reloading");
                reload_from_channel(state, path).await;
            }
            // （6.1）对端多了一种本 build 不认识的帧: 忽略它并把流留住——
            // 未知帧不该让 router 永久断连（旧行为是解析失败 → 重连）。
            StateStreamFrame::Unknown => {
                tracing::debug!("core channel sent an unknown frame; ignoring it");
            }
        }
    }
}
