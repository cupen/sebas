//! 测试支撑：启动真实 gateway（OS 分配端口），自动 set 两个测试 env key，
//! 并把 config 中的 `__USAGE__` 占位替换为 tempdir 内 usage.jsonl，
//! 避免测试污染 `~/.local/state`。
//!
//! Task 9 会扩展本模块追加 `start_mock_upstream`。

use std::net::SocketAddr;
use std::sync::Once;

use tempfile::TempDir;

use gateway::config::GatewayConfig;
use gateway::server;

/// 启动一个 gateway 实例并返回其监听地址 + 持有 TempDir（drop 即清理）。
///
/// `config_toml` 中：
/// - `usage_file = "__USAGE__"` 会被替换为 tempdir 内 `usage.jsonl`；
/// - provider 的 `api_key_env` 应指向 `SEBAS_GATEWAY_TEST_UPSTREAM_KEY`
///   或 `SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI`，本函数自动 set 两者。
///
/// Task 8 的 usage sink 会写经 `__USAGE__` 替换出的 tempdir 路径，故测试
/// 不会触及 `~/.local/state`。
pub async fn start_gateway(config_toml: &str) -> TestGateway {
    ensure_test_env_keys();

    let dir = tempfile::tempdir().expect("tempdir");
    let usage_path = dir.path().join("usage.jsonl");
    let raw = config_toml.replace("__USAGE__", &usage_path.to_string_lossy());
    let cfg = GatewayConfig::parse(&raw).expect("parse test config");
    let state = server::build_state(cfg).expect("build_state");
    let app = server::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server ran");
    });

    TestGateway {
        addr,
        dir,
        _server: server,
    }
}

/// 运行中的 gateway 测试实例。drop 时 abort 后台 task + 清理 TempDir。
pub struct TestGateway {
    pub addr: SocketAddr,
    /// 持有以保持 tempdir 存活至 drop；Task 9 会读 `dir.path()` 轮询 usage.jsonl。
    /// 本测试二进制未直接读，故 `allow(dead_code)`。
    #[allow(dead_code)]
    pub dir: TempDir,
    _server: tokio::task::JoinHandle<()>,
}

impl Drop for TestGateway {
    fn drop(&mut self) {
        self._server.abort();
    }
}

static ENV_ONCE: Once = Once::new();

/// 设置两个测试上游 key（每个测试进程仅 set 一次，值恒定）。
///
/// 用 `Once` 而非每次 `start_gateway` 都 set：本进程内多个 `#[tokio::test]`
/// 并发调用 `start_gateway` 时，`call_once` 保证 set 恰好发生一次且先于任何
/// `build_state` 的 `std::env::var` 读取返回，无写读竞态。
fn ensure_test_env_keys() {
    ENV_ONCE.call_once(|| {
        // SAFETY: `Once::call_once` 保证本块在进程内只执行一次；set 后不 remove、
        // 值恒定。各测试文件独立进程，无跨文件竞态。后续 `build_state` 的
        // `std::env::var` 读取发生在 `call_once` 返回之后，无写读竞态。
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_TEST_UPSTREAM_KEY", "test-anthropic-key");
            std::env::set_var("SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI", "test-openai-key");
        }
    });
}
