//! 核心通道状态订阅客户端 (add-state-store 5.3)。
//!
//! 当 core channel 本地 socket 可用时, 订阅状态变更通知, 收到通知后触发
//! provider/alias 热重载。通道不可用时降级为文件监听 (hot_reload 保持)。
//!
//! 协议: NDJSON over 本地 IPC (Unix socket / Windows named pipe), 与
//! `src/core_channel/protocol.rs` 同规范。握手带 `SEBAS_CORE_SECRET`
//! (watchdog 注入, 与 core/webui 同密钥); 订阅是持久流——先一帧全域快照,
//! 之后每帧一条 scope 变更(一串提交已由服务端合并)。断连时保持最后有效
//! 配置, 由 `ReloadStatus` 记录不可用状态供 `/admin/stats` 暴露。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use crate::server::AppState;

/// 核心通道 socket 路径, 由 `SEBAS_CORE_SOCKET` 环境变量指定。
/// 未设置时返回 `None` (通道不可用, 走文件监听)。
pub(crate) fn socket_path() -> Option<PathBuf> {
    let raw = std::env::var("SEBAS_CORE_SOCKET").ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(raw))
}

/// 握手 secret 解析 (harden-core-channel-deployment D2): `SEBAS_CORE_SECRET`
/// env 优先 (watchdog 注入, 零开销); 缺失时读 secret 文件 (watchdog 经
/// `SEBAS_CORE_SECRET_FILE` 把与 core 同一份 `-c` config 解析出的路径 pin
/// 给 router 子进程) —— core 重启换钥后订阅循环在退避重连时天然拿到新钥。
/// 两者皆缺省返回空串 (不断言、不崩溃; 握手失败走既有的重连退避)。
pub(crate) fn channel_secret() -> String {
    if let Ok(v) = std::env::var("SEBAS_CORE_SECRET")
        && !v.is_empty()
    {
        return v;
    }
    if let Ok(p) = std::env::var("SEBAS_CORE_SECRET_FILE")
        && !p.is_empty()
        && let Ok(raw) = std::fs::read_to_string(&p)
    {
        let secret = raw.trim().to_string();
        if !secret.is_empty() {
            return secret;
        }
    }
    String::new()
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
/// 返回完整响应（`CoreChannelResponse` 形状）。请求/响应均为 NDJSON on 本地 IPC。
async fn channel_request(
    path: &Path,
    req: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stream = sebas_ipc::connect(path)
        .await
        .map_err(|e| format!("core channel connect failed: {e}"))?;
    let (reader, mut writer) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(reader);

    // 握手: 带解析出的 secret (env 优先, 否则 secret 文件)。
    let secret = channel_secret();
    let hs = serde_json::json!({"secret": secret});
    let mut line = serde_json::to_string(&hs).map_err(|e| format!("handshake serialize failed: {e}"))?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
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

    let mut req_line =
        serde_json::to_string(req).map_err(|e| format!("request serialize failed: {e}"))?;
    req_line.push('\n');
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
    let resp_line = resp.trim();
    serde_json::from_str(resp_line).map_err(|e| format!("response parse failed: {e}"))
}

/// 短连接请求 core 的 `StateSnapshot{domain}`，返回 payload。
/// 订阅触发 reload 时用于从通道拉最新 providers/aliases（5.3 投影）。
pub(crate) async fn fetch_state_snapshot(
    path: &Path,
    domain: &str,
) -> Result<serde_json::Value, String> {
    let req = serde_json::json!({"cmd": "state_snapshot", "domain": domain});
    let resp = channel_request(path, &req).await?;
    #[derive(Deserialize)]
    struct SnapshotResp {
        cmd: String,
        payload: serde_json::Value,
    }
    let parsed: SnapshotResp = serde_json::from_value(resp)
        .map_err(|e| format!("snapshot response parse failed: {e}"))?;
    if parsed.cmd != "state_snapshot" {
        return Err(format!("unexpected response cmd: {}", parsed.cmd));
    }
    Ok(parsed.payload)
}

/// 短连接 core `StateMutation{domain}`。成功 → Ok；被拒 → Err(成因)。
pub(crate) async fn mutate_state(
    path: &Path,
    domain: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    let req = serde_json::json!({"cmd": "state_mutation", "domain": domain, "payload": payload});
    let resp = channel_request(path, &req).await?;
    if resp.get("cmd").and_then(serde_json::Value::as_str) == Some("state_mutation_ok") {
        return Ok(());
    }
    // Rejected：提取 cause（与 core channel 的 SessionRejection::Unavailable
    // 形状一致：`{"cmd":"rejected", "rejection":{"cause":"..."}}`）。
    let cause = resp
        .get("rejection")
        .and_then(|r| r.get("cause"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("state mutation rejected");
    Err(cause.to_string())
}

/// 状态订阅流的 wire 帧 (与 core 侧 `StateStreamFrame` 对齐的 subset)。
#[derive(Debug, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case")]
enum StateStreamFrame {
    /// 快照帧: 载荷仅用于「连接已就绪」的信号, 内容不消费
    /// (reload 会重新读 overlay/config)。
    #[allow(dead_code)]
    Snapshot { domains: serde_json::Value },
    Changed { scope: String },
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

    // 握手: 带解析出的 secret (env 优先, 否则 secret 文件)。
    let secret = channel_secret();
    let hs = serde_json::json!({"secret": secret});
    let mut line = serde_json::to_string(&hs).map_err(|e| format!("handshake serialize failed: {e}"))?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("handshake write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("handshake flush failed: {e}"))?;

    // 读握手 ack
    let mut ack = String::new();
    reader
        .read_line(&mut ack)
        .await
        .map_err(|_| "handshake ack read failed (secret may have been rejected)".to_string())?;
    if ack.trim().is_empty() {
        return Err("handshake ack empty".into());
    }

    // 发送 StateSubscribe 请求
    let req = serde_json::json!({"cmd": "state_subscribe"});
    let mut req_line = serde_json::to_string(&req).map_err(|e| format!("request serialize failed: {e}"))?;
    req_line.push('\n');
    writer
        .write_all(req_line.as_bytes())
        .await
        .map_err(|e| format!("request write failed: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("request flush failed: {e}"))?;

    // 读帧循环: 先快照, 之后持续读变更通知。
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
        let frame: StateStreamFrame = serde_json::from_str(line.trim())
            .map_err(|e| format!("frame parse failed: {e}"))?;
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
        }
    }
}
