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

/// 启动核心通道订阅循环 (tokio task)。
/// 当 socket 路径可用时, 连接并订阅状态变更; 不可用时静默返回。
pub fn spawn_subscriber(state: AppState) {
    let Some(path) = socket_path() else {
        tracing::info!("core channel socket 未配置 (SEBAS_CORE_SOCKET), 使用文件监听");
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
    loop {
        // 每次尝试前标记数据源不可用 (重连成功后由 subscribe_once 清除)。
        state
            .reload_status
            .record_source_unavailable("core channel 断连, 保持最后有效配置");
        match subscribe_once(&state, &path).await {
            Ok(()) => {
                // 正常断开 (core 关闭通道后重连)。
                backoff = Duration::from_secs(1);
            }
            Err(e) => {
                tracing::warn!(error = %e, "core channel 订阅失败, 退避 {backoff:?}");
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
        state.swap_core(cfg).map_err(|e| format!("热替换失败: {e}"))?;
        Ok::<(), String>(())
    }
    .await;
    match result {
        Ok(()) => {
            state.reload_status.record_source_ok();
            state.reload_status.record_ok_quiet();
            tracing::info!("core channel 快照投影成功, 配置已热替换");
        }
        Err(e) => {
            tracing::warn!("core channel 快照投影失败（保旧内核）: {e}");
            state.reload_status.record_err(&e);
            // 快照拉取失败但通道仍活着（如 parse 错误）→ 尝试文件回退。
            if e.contains("连接 core channel 失败") || e.contains("握手") || e.contains("响应读取失败")
            {
                tracing::info!("core channel 不可用, 回退文件 overlay 重载");
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
        .map_err(|e| format!("连接 core channel 失败: {e}"))?;
    let (reader, mut writer) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(reader);

    // 握手: 带 `SEBAS_CORE_SECRET` (watchdog 注入, 与 core/webui 同密钥)。
    let secret = std::env::var("SEBAS_CORE_SECRET").unwrap_or_default();
    let hs = serde_json::json!({"secret": secret});
    let mut line = serde_json::to_string(&hs).map_err(|e| format!("序列化握手失败: {e}"))?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("握手写入失败: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("握手 flush 失败: {e}"))?;

    let mut ack = String::new();
    reader
        .read_line(&mut ack)
        .await
        .map_err(|_| "握手 ack 读取失败 (secret 可能被拒绝)".to_string())?;
    if ack.trim().is_empty() {
        return Err("握手 ack 为空".into());
    }

    let mut req_line =
        serde_json::to_string(req).map_err(|e| format!("序列化请求失败: {e}"))?;
    req_line.push('\n');
    writer
        .write_all(req_line.as_bytes())
        .await
        .map_err(|e| format!("请求写入失败: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("请求 flush 失败: {e}"))?;

    let mut resp = String::new();
    reader
        .read_line(&mut resp)
        .await
        .map_err(|_| "响应读取失败".to_string())?;
    let resp_line = resp.trim();
    serde_json::from_str(resp_line).map_err(|e| format!("解析响应失败: {e}"))
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
        .map_err(|e| format!("解析快照响应失败: {e}"))?;
    if parsed.cmd != "state_snapshot" {
        return Err(format!("意外响应 cmd: {}", parsed.cmd));
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
        .map_err(|e| format!("连接 core channel 失败: {e}"))?;
    let (reader, mut writer) = sebas_ipc::split(stream);
    let mut reader = BufReader::new(reader);

    // 握手: 带 `SEBAS_CORE_SECRET` (watchdog 注入, 与 core/webui 同密钥)。
    let secret = std::env::var("SEBAS_CORE_SECRET").unwrap_or_default();
    let hs = serde_json::json!({"secret": secret});
    let mut line = serde_json::to_string(&hs).map_err(|e| format!("序列化握手失败: {e}"))?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("握手写入失败: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("握手 flush 失败: {e}"))?;

    // 读握手 ack
    let mut ack = String::new();
    reader
        .read_line(&mut ack)
        .await
        .map_err(|_| "握手 ack 读取失败 (secret 可能被拒绝)".to_string())?;
    if ack.trim().is_empty() {
        return Err("握手 ack 为空".into());
    }

    // 发送 StateSubscribe 请求
    let req = serde_json::json!({"cmd": "state_subscribe"});
    let mut req_line = serde_json::to_string(&req).map_err(|e| format!("序列化请求失败: {e}"))?;
    req_line.push('\n');
    writer
        .write_all(req_line.as_bytes())
        .await
        .map_err(|e| format!("请求写入失败: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("请求 flush 失败: {e}"))?;

    // 读帧循环: 先快照, 之后持续读变更通知。
    let mut line = String::new();
    let mut got_snapshot = false;
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("流读取失败: {e}"))?;
        if n == 0 {
            return Err("connection dropped".into());
        }
        let frame: StateStreamFrame = serde_json::from_str(line.trim())
            .map_err(|e| format!("解析帧失败: {e}"))?;
        match frame {
            StateStreamFrame::Snapshot { .. } => {
                // 订阅成功: 连接健康, 清除数据源不可用并触发一次完全 reload。
                got_snapshot = true;
                state.reload_status.record_source_ok();
                tracing::info!("core channel 状态订阅成功, 触发 provider 重载");
                reload_from_channel(state, path).await;
            }
            StateStreamFrame::Changed { scope } => {
                if !got_snapshot {
                    // 未见快照先见变更, 协议外但可容错: 依旧触发 reload。
                    tracing::warn!("未收到快照先收到变更({scope})");
                }
                tracing::info!("core channel 状态变更: scope={scope}, 触发重载");
                reload_from_channel(state, path).await;
            }
        }
    }
}
