//! 网关 HTTP access log（nginx access log 风格，一行紧凑输出）。
//!
//! 挂在最外层中间件，覆盖 `/healthz` 与全部透传请求。每个请求完成后（或
//! SSE 流结束 / 客户端断开时）写一行日志，形如：
//! `127.0.0.1 - [2026-08-09T15:44:11Z] "POST /v1/messages" deepseek-chat@deepseek 200 123 1ms`
//!
//! model / provider 由 proxy 在路由解析后回填（`AccessLogHandle`）；未到 proxy
//! 的请求（如 401）记为 `-`。日志直接走 `tracing` 写到标准输出（gateway_cmd 的
//! init_tracing 默认 stdout），不做文件/轮转。target 为 `gateway::access`。

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;
use futures_core::Stream;

/// 一次请求的 access log 条目。`bytes` 由 `LoggedBody` 累计，`model` 由
/// proxy 回填；流结束/断开时由 `Drop` 写出（对标 nginx 在请求完成时记日志，
/// 客户端中断按已发字节记）。
struct AccessLogEntry {
    start: Instant,
    ip: String,
    method: String,
    path: String,
    model: String,
    provider: String,
    status: u16,
    bytes: u64,
}

impl AccessLogEntry {
    fn write(&self) {
        let route = if self.model == "-" && self.provider == "-" {
            "-".to_string()
        } else {
            format!("{}@{}", self.model, self.provider)
        };
        tracing::info!(
            target: "gateway::access",
            "{} - [{}] \"{} {}\" {} {} {} {}ms",
            self.ip,
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            self.method,
            self.path,
            route,
            self.status,
            self.bytes,
            self.start.elapsed().as_millis(),
        );
    }
}

/// 共享条目句柄：中间件创建并注入 request extensions，proxy 解析出 model 后
/// 回填（`set_model`），响应体结束时由 `LoggedBody` 读取并写日志。
#[derive(Clone)]
pub struct AccessLogHandle(Arc<Mutex<AccessLogEntry>>);

impl AccessLogHandle {
    pub fn set_model(&self, model: &str) {
        if let Ok(mut e) = self.0.lock() {
            e.model = model.to_string();
        }
    }

    pub fn set_provider(&self, provider: &str) {
        if let Ok(mut e) = self.0.lock() {
            e.provider = provider.to_string();
        }
    }
}

/// 响应体包装：透传 chunk 并累计已下发字节；流被消费完或 drop 时写 access log。
struct LoggedBody<S> {
    inner: S,
    entry: Arc<Mutex<AccessLogEntry>>,
}

impl<S, E> Stream for LoggedBody<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    type Item = Result<Bytes, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if let Ok(mut e) = self.entry.lock() {
                    e.bytes += bytes.len() as u64;
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            other => other,
        }
    }
}

impl<S> Drop for LoggedBody<S> {
    fn drop(&mut self) {
        if let Ok(e) = self.entry.lock() {
            e.write();
        }
    }
}

/// access log 中间件（挂在最外层，先于鉴权执行，覆盖所有路由与 fallback）。
pub async fn access_log(req: Request, next: Next) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string())
        .unwrap_or_else(|| "-".to_string());
    let method = req.method().to_string();
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let entry = Arc::new(Mutex::new(AccessLogEntry {
        start: Instant::now(),
        ip,
        method,
        path,
        model: "-".into(),
        provider: "-".into(),
        status: 0,
        bytes: 0,
    }));

    let mut req = req;
    req.extensions_mut().insert(AccessLogHandle(entry.clone()));
    let resp = next.run(req).await;
    if let Ok(mut e) = entry.lock() {
        e.status = resp.status().as_u16();
    }
    resp.map(|body| {
        let stream = body.into_data_stream();
        axum::body::Body::from_stream(LoggedBody {
            inner: stream,
            entry: entry.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn logged_body_counts_bytes_and_writes_on_drop() {
        // 无法在单测里断言 tracing 输出，但可以验证字节计数与 drop 路径
        // 不 panic（Drop 里写日志）。
        let entry = Arc::new(Mutex::new(AccessLogEntry {
            start: Instant::now(),
            ip: "127.0.0.1".into(),
            method: "GET".into(),
            path: "/v1/models".into(),
            model: "-".into(),
            provider: "-".into(),
            status: 200,
            bytes: 0,
        }));
        let chunks: Vec<Result<Bytes, std::convert::Infallible>> = vec![
            Ok(Bytes::from_static(b"hello ")),
            Ok(Bytes::from_static(b"world")),
        ];
        let chunks = stream::iter(chunks);
        let mut logged = LoggedBody {
            inner: chunks,
            entry: entry.clone(),
        };
        use futures_util::StreamExt;
        let collected: Vec<Bytes> = logged
            .by_ref()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(collected.len(), 2);
        assert_eq!(entry.lock().unwrap().bytes, 11);
    }

    #[test]
    fn handle_set_model_updates_shared_entry() {
        let entry = Arc::new(Mutex::new(AccessLogEntry {
            start: Instant::now(),
            ip: "127.0.0.1".into(),
            method: "POST".into(),
            path: "/v1/messages".into(),
            model: "-".into(),
            provider: "-".into(),
            status: 200,
            bytes: 0,
        }));
        let handle = AccessLogHandle(entry.clone());
        handle.set_model("deepseek-chat");
        handle.set_provider("deepseek");
        assert_eq!(entry.lock().unwrap().model, "deepseek-chat");
        assert_eq!(entry.lock().unwrap().provider, "deepseek");
    }
}
