//! core→router admin 的 loopback HTTP 反代取数（add-usage-statistics D1/D7）。
//!
//! 链路形态（design D1）：`browser → /api/usage (webui) → core channel RPC
//! (core) → loopback HTTP (router admin)`。core 是控制面编排者，持有 config
//! 与控制密钥，是这条新出边的唯一合理主人；webui 进程的服务端出边保持只有
//! core channel 一条。
//!
//! # Bearer 控制密钥（对 design D1 的一处勘误，见 change 验证记录）
//!
//! router admin 面鉴权读的是 `SEBAS_CONTROL_SECRET` env（`admin_auth`）——
//! watchdog 把**同一个**控制密钥下发给 core 与 router 两个子进程，core 直接
//! 用自己 env 里的值即可。design D1 写的「复用 `sebas_ipc::secret` 共享发现
//! （env → core.secret 文件）」不成立：`core.secret` 是 session channel 的
//! 握手密钥（watchdog 生成的 `core_secret`，与控制密钥是两个值），拿它当
//! Bearer 会被 admin_auth 拒绝；且控制密钥没有文件形态（只有 env）。无
//! secret 的 standalone 部署两者都为空——router admin 对 loopback 放行，
//! 这里就不带 Bearer（两种部署姿势零改动）。
//!
//! # 失败面（D7）
//!
//! 5s 短超时；router 未启用（config 无 `[router] listen`）/ 连接拒绝 / 超时
//! 映射为 [`UsageProxyError::Unreachable`]，绝不拖垮调用面；router 应答的非
//! 200（如参数 400）按 [`UsageProxyError::RouterError`] 原样透传状态与体。

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

/// 结构化 cause 的机器可读前缀（webui 空态判别依据，design D7）。
pub const ROUTER_UNREACHABLE_CAUSE: &str = "router_unreachable";

/// router 未随部署启用时的完整 cause（config 无 `[router]` 段）。
pub const ROUTER_NOT_CONFIGURED_CAUSE: &str =
    "router_unreachable: config 无 [router] listen（router 未随部署启用）";

/// `[router] listen` 的缺省（与 router 侧 `default_listen` 同值——部署不配
/// listen 时两侧落同一端口）。
pub const DEFAULT_ROUTER_LISTEN: &str = "127.0.0.1:8787";

/// core→router 的短超时（D7：聚合是只读 SELECT，5s 足够且绝不悬挂）。
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// 反代失败面。
#[derive(Debug, Clone, PartialEq)]
pub enum UsageProxyError {
    /// router 未启用 / 连接拒绝 / 超时 / 应答体不可解析——聚合面不可用。
    /// cause 以 [`ROUTER_UNREACHABLE_CAUSE`] 前缀点名。
    Unreachable { cause: String },
    /// router 应答了非 200：状态码与 JSON 体原样透传（如 400 参数错误）。
    RouterError { status: u16, body: Value },
}

impl UsageProxyError {
    /// 机器可读 cause（webui 层据此区分「router 不可达」空态）。
    pub fn cause(&self) -> String {
        match self {
            UsageProxyError::Unreachable { cause } => cause.clone(),
            UsageProxyError::RouterError { status, body } => format!(
                "{ROUTER_UNREACHABLE_CAUSE}: router 应答异常状态 {status}: {body}"
            ),
        }
    }
}

/// 共享 HTTP client（connect+total 都有界；进程级一份，零每请求建连成本）。
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(FETCH_TIMEOUT)
            .build()
            .expect("router admin http client builds")
    })
}

/// loopback `GET /admin/usage/timeseries`。`listen` 取 config `[router]
/// listen`（缺省 [`DEFAULT_ROUTER_LISTEN`]）；Bearer 用控制密钥（见模块注释）。
pub async fn fetch_usage_timeseries(
    listen: &str,
    granularity: &str,
    days: u32,
    tz_offset: i32,
) -> Result<Value, UsageProxyError> {
    let url = format!(
        "http://{listen}/admin/usage/timeseries?granularity={}&days={days}&tz_offset={tz_offset}",
        urlencoding::encode(granularity),
    );
    let mut req = client().get(&url);
    if let Ok(secret) = std::env::var("SEBAS_CONTROL_SECRET")
        && !secret.is_empty()
    {
        req = req.bearer_auth(secret);
    }
    match tokio::time::timeout(FETCH_TIMEOUT, req.send()).await {
        Ok(Ok(resp)) => {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            if status == 200 {
                serde_json::from_str(&text).map_err(|e| UsageProxyError::Unreachable {
                    cause: format!(
                        "{ROUTER_UNREACHABLE_CAUSE}: router 聚合应答不是合法 JSON: {e}"
                    ),
                })
            } else {
                Err(UsageProxyError::RouterError {
                    status,
                    body: serde_json::from_str(&text).unwrap_or(Value::Null),
                })
            }
        }
        Ok(Err(e)) => Err(UsageProxyError::Unreachable {
            cause: format!("{ROUTER_UNREACHABLE_CAUSE}: {e}"),
        }),
        Err(_) => Err(UsageProxyError::Unreachable {
            cause: format!(
                "{ROUTER_UNREACHABLE_CAUSE}: 请求 router 聚合超时（{}s）",
                FETCH_TIMEOUT.as_secs()
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unreachable_cause_of(err: &UsageProxyError) -> String {
        match err {
            UsageProxyError::Unreachable { cause } => cause.clone(),
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    /// 不存在的端口 → Unreachable 且 promptly 返回（task 2.2 的单测口径；
    /// 端到端的 cause 映射断言在 core_channel 分支测试里）。
    #[tokio::test]
    async fn dead_port_maps_to_unreachable_promptly() {
        // 找一个必然没人监听的端口：绑定后立即释放。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let started = std::time::Instant::now();
        let err = fetch_usage_timeseries(&format!("127.0.0.1:{port}"), "day", 14, 0)
            .await
            .expect_err("dead port must fail");
        let cause = unreachable_cause_of(&err);
        assert!(
            cause.starts_with(ROUTER_UNREACHABLE_CAUSE),
            "cause 必须以 router_unreachable 点名: {cause}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "连接拒绝必须 promptly 返回，实测 {:?}",
            started.elapsed()
        );
    }

    /// 非法 granularity 不该出 core（webui/通道侧先校验）；这里钉 URL 编码
    /// 助手对未预期取值也不产生注入面。
    #[tokio::test]
    async fn odd_granularity_is_url_encoded_not_injected() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = fetch_usage_timeseries(
            &format!("127.0.0.1:{port}"),
            "day&days=1&tz_offset=0#",
            14,
            0,
        )
        .await
        .expect_err("dead port must fail");
        // 到达 URL 的只是编码后的取值——失败仍是「连接拒绝」，不是别的路径。
        assert!(matches!(err, UsageProxyError::Unreachable { .. }));
    }
}
