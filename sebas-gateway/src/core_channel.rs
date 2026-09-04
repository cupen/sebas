//! 核心通道状态订阅客户端 (add-state-store 5.3)。
//!
//! 当 core channel Unix socket 可用时, 订阅状态变更通知, 收到通知后触发
//! provider/alias 热重载。通道不可用时降级为文件监听 (hot_reload 保持)。
//!
//! 协议: NDJSON over Unix socket, 与 `src/core_channel/protocol.rs` 同规范。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::server::AppState;

/// 核心通道 Unix socket 路径, 由 `SEBAS_CORE_SOCKET` 环境变量指定。
/// 未设置时返回 `None` (通道不可用, 走文件监听)。
fn socket_path() -> Option<PathBuf> {
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
/// 断连时退避重连。
async fn subscribe_loop(state: AppState, path: PathBuf) {
    let mut backoff = Duration::from_secs(1);
    loop {
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

/// 一次完整的订阅会话: 连接 → 握手 → 发送请求 → 接收响应 → 触发 reload。
async fn subscribe_once(state: &AppState, path: &PathBuf) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(path)
        .await
        .map_err(|e| format!("连接 core channel 失败: {e}"))?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // 握手: 发送空 secret (本地 same-uid 连接, uid 鉴权已足够)。
    let hs = serde_json::json!({"secret": ""});
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

    // 读响应: 期望 StateSnapshot 帧
    let mut resp = String::new();
    reader
        .read_line(&mut resp)
        .await
        .map_err(|_| "响应读取失败".to_string())?;
    let resp_line = resp.trim();

    // 检查响应是否成功 (StateSnapshot 或 StateMutationOk)
    if resp_line.contains("\"cmd\":\"state_snapshot\"") || resp_line.contains("\"cmd\":\"state_mutation_ok\"") {
        // 订阅成功, 触发一次完全 reload。
        tracing::info!("core channel 状态订阅成功, 触发 provider 重载");
        let _ = crate::admin::reload_and_swap(state);
        // 会话结束: 单次订阅后断开 (StateSubscribe 不是持久流)。
        // gateway 每次收到外部通知时重新连接获取最新状态。
        // 当前实现: 每次订阅后断开, 由上层循环重连。
        Ok(())
    } else {
        // 未知响应, 记录但不算致命。
        tracing::warn!(resp = %resp_line, "core channel 订阅返回意外响应");
        Ok(())
    }
}