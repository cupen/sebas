//! 进程级 e2e：spawn 真实 `sebas gateway` 二进制，provider 指向 in-process
//! mock upstream，复用 `support` 的 fixture 做字节级断言。
//!
//! 覆盖：anthropic messages JSON/SSE、openai chat JSON/SSE 字节级透传、
//! 401 鉴权、usage.jsonl 至少一条 record、上游收到注入的 key。
//!
//! binary 定位：gateway crate 在 workspace 子目录，`target/debug` 在 workspace
//! 根（上一级）。binary 缺失时 skip（不失败）——本地先 `cargo build --bin sebas`；
//! CI 的 test job 已显式 build，故 CI 上必然真跑。

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use gateway::proto::WireProtocol;
use support::*;

/// 定位 workspace 根 `target/debug`（gateway crate 的 manifest 在其子目录）。
fn workspace_target_debug() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set during cargo test");
    PathBuf::from(manifest_dir)
        .join("..")
        .join("target")
        .join("debug")
}

fn sebas_bin() -> PathBuf {
    let name = if cfg!(windows) { "sebas.exe" } else { "sebas" };
    workspace_target_debug().join(name)
}

/// bind-then-drop 选一个空闲端口（进程启动前端口已释放，竞态极小）。
async fn pick_free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);
    addr.port()
}

/// 写临时 gateway config：双 provider 指向两个 mock，usage 落到 target/tests/。
/// 返回 config 路径（TOML 里路径统一用 `/`，兼容 Windows 反斜杠转义问题）。
fn write_config(
    dir: &Path,
    port: u16,
    anth_url: &str,
    oai_url: &str,
    usage_path: &Path,
) -> PathBuf {
    let usage = usage_path.to_string_lossy().replace('\\', "/");
    let path = dir.join("gateway.toml");
    let body = format!(
        r#"
[gateway]
listen = "127.0.0.1:{port}"
usage_file = "{usage}"
# 隔离：不合并开发机 ~/.sebas/providers.json（其 openai 条目与 preset
# 校验冲突会让 gateway 启动即失败）。
provider_overlay = "{}/no-overlay.json"
default_provider = "anthropic"

auth_token = "sk-gw-process"

[gateway.routes]
"claude-*" = ["anthropic"]
"gpt-*" = ["openai"]

[provider.anthropic]
base_url_anthropic = "{anth_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[provider.openai]
base_url_openai = "{oai_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI"
"#,
        dir.to_string_lossy().replace('\\', "/")
    );
    std::fs::write(&path, body).expect("write gateway config");
    path
}

/// 独立 `sebas gateway` 进程 + 顶层 `[provider.*]` 的配置：`[gateway]` 只放
/// listen/auth_token/usage，provider 定义在顶层（与 run 共用）。protocol 省略 →
/// preset 自动填 anthropic；base_url 显式指向 mock；api_key_env 显式。
fn write_top_level_provider_config(
    dir: &Path,
    port: u16,
    anth_url: &str,
    usage_path: &Path,
) -> PathBuf {
    let usage = usage_path.to_string_lossy().replace('\\', "/");
    let path = dir.join("gateway-top-level.toml");
    let body = format!(
        r#"
[gateway]
listen = "127.0.0.1:{port}"
usage_file = "{usage}"
# 隔离：不合并开发机 ~/.sebas/providers.json（其 openai 条目与 preset
# 校验冲突会让 gateway 启动即失败）。
provider_overlay = "{}/no-overlay.json"

auth_token = "sk-gw-top"

[provider.anthropic]
base_url_anthropic = "{anth_url}"
api_key_env = "ANTHROPIC_API_KEY"
"#,
        dir.to_string_lossy().replace('\\', "/")
    );
    std::fs::write(&path, body).expect("write top-level-provider gateway config");
    path
}

/// 运行中的 gateway 子进程。drop 时 kill + wait，不泄漏进程。
struct GatewayProcess {
    child: Child,
}

impl GatewayProcess {
    /// 失败路径专用：kill → wait → 读回 stderr 供诊断。
    fn kill_and_capture_stderr(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut buf = String::new();
        if let Some(mut e) = self.child.stderr.take() {
            use std::io::Read;
            let _ = e.read_to_string(&mut buf);
        }
        buf
    }
}

impl Drop for GatewayProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

const DOWNSTREAM_KEY: &str = "sk-gw-process";

#[tokio::test]
async fn real_binary_forwards_anthropic_openai_auth_and_usage() {
    let bin = sebas_bin();
    if !bin.exists() {
        eprintln!(
            "skipping: {} missing (run `cargo build --bin sebas` first)",
            bin.display()
        );
        return;
    }

    let anth = start_mock_upstream(WireProtocol::Anthropic).await;
    let oai = start_mock_upstream(WireProtocol::OpenAi).await;
    let dir = support::test_target_dir("process_e2e");
    let usage_path = dir.path().join("usage.jsonl");
    let port = pick_free_port().await;
    let config_path = write_config(dir.path(), port, &anth.url, &oai.url, &usage_path);

    let mut gw = GatewayProcess {
        child: Command::new(&bin)
            .arg("gateway")
            .arg("--config")
            .arg(&config_path)
            .env("SEBAS_GATEWAY_TEST_UPSTREAM_KEY", "test-anthropic-key")
            .env("SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI", "test-openai-key")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sebas gateway"),
    };
    let base = format!("http://127.0.0.1:{port}");

    // 轮询 /healthz 直到就绪；进程提前退出 / 超时 → dump stderr 失败。
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        let client = reqwest::Client::new();
        loop {
            let ok = client
                .get(format!("{base}/healthz"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ok {
                break;
            }
            if let Some(status) = gw.child.try_wait().expect("try_wait") {
                panic!(
                    "sebas gateway exited early with {status}: {}",
                    gw.kill_and_capture_stderr()
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if ready.is_err() {
        panic!(
            "sebas gateway not ready within 10s; stderr: {}",
            gw.kill_and_capture_stderr()
        );
    }

    let client = client();
    let auth = ("authorization", format!("Bearer {DOWNSTREAM_KEY}"));
    let auth_val = auth.1.as_str();

    // 1. anthropic messages 非流式 JSON 字节级透传
    let resp = client
        .post(format!("{base}/v1/messages"))
        .header(auth.0, auth_val)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-mock-trace").unwrap(),
        "anthropic-messages-json"
    );
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    let bytes = resp.bytes().await.expect("read body");
    assert_eq!(&bytes[..], ANTHROPIC_MESSAGES_JSON.as_bytes());

    // 2. anthropic messages SSE 字节级透传
    let resp = client
        .post(format!("{base}/v1/messages"))
        .header(auth.0, auth_val)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}],"stream":true}"#)
        .send()
        .await
        .expect("POST /v1/messages stream");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-mock-trace").unwrap(),
        "anthropic-messages-sse"
    );
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let bytes = resp.bytes().await.expect("read SSE body");
    assert_eq!(&bytes[..], ANTHROPIC_MESSAGES_SSE.as_bytes());

    // 3. openai chat 非流式 JSON 字节级透传
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .header(auth.0, auth_val)
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .expect("POST /v1/chat/completions");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-mock-trace").unwrap(),
        "openai-chat-json"
    );
    let bytes = resp.bytes().await.expect("read body");
    assert_eq!(&bytes[..], OPENAI_CHAT_JSON.as_bytes());

    // 4. openai chat SSE 字节级透传
    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .header(auth.0, auth_val)
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}],"stream":true}"#)
        .send()
        .await
        .expect("POST /v1/chat/completions stream");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-mock-trace").unwrap(),
        "openai-chat-sse"
    );
    let bytes = resp.bytes().await.expect("read SSE body");
    assert_eq!(&bytes[..], OPENAI_CHAT_SSE.as_bytes());

    // 5. 无 key → 401
    let resp = client
        .post(format!("{base}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST without key");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 6. usage.jsonl 至少一条 record（前四次 200 调用任意一条即可）
    let records = poll_usage_jsonl(&usage_path, 1).await;
    // 无 per-key 身份：key 恒为空（绝不写 token 本体）。
    assert_eq!(records[0]["key"], "");
    assert_eq!(records[0]["status"], 200);

    // 7. mock 收到注入的上游 key（child 真转发，非仅返回 fixture）
    let anth_reqs = anth.requests.lock().await;
    assert!(
        anth_reqs
            .iter()
            .any(|r| recorded_header_get(&r.headers, "x-api-key") == Some("test-anthropic-key")),
        "anthropic mock must receive injected x-api-key"
    );
    let oai_reqs = oai.requests.lock().await;
    assert!(
        oai_reqs.iter().any(|r| {
            recorded_header_get(&r.headers, "authorization") == Some("Bearer test-openai-key")
        }),
        "openai mock must receive injected Bearer"
    );

    // 8. 正常路径：kill + wait（Drop 兜底，这里显式收尾便于观察）
    let stderr = gw.kill_and_capture_stderr();
    if !stderr.is_empty() {
        eprintln!("sebas gateway stderr:\n{stderr}");
    }
}

#[tokio::test]
async fn standalone_gateway_reads_top_level_provider_table() {
    let bin = sebas_bin();
    if !bin.exists() {
        eprintln!(
            "skipping: {} missing (run `cargo build --bin sebas` first)",
            bin.display()
        );
        return;
    }

    let anth = start_mock_upstream(WireProtocol::Anthropic).await;
    let dir = support::test_target_dir("process_e2e_top");
    let usage_path = dir.path().join("usage.jsonl");
    let port = pick_free_port().await;
    let config_path = write_top_level_provider_config(dir.path(), port, &anth.url, &usage_path);

    let mut gw = GatewayProcess {
        child: Command::new(&bin)
            .arg("gateway")
            .arg("--config")
            .arg(&config_path)
            .env("ANTHROPIC_API_KEY", "test-anthropic-key")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn standalone sebas gateway"),
    };
    let base = format!("http://127.0.0.1:{port}");

    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        let client = reqwest::Client::new();
        loop {
            let ok = client
                .get(format!("{base}/healthz"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ok {
                break;
            }
            if let Some(status) = gw.child.try_wait().expect("try_wait") {
                panic!(
                    "sebas gateway exited early with {status}: {}",
                    gw.kill_and_capture_stderr()
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    if ready.is_err() {
        panic!(
            "sebas gateway not ready within 10s; stderr: {}",
            gw.kill_and_capture_stderr()
        );
    }

    let client = client();
    // 顶层 provider + preset（protocol 省略）→ 字节级透传
    let resp = client
        .post(format!("{base}/v1/messages"))
        .header("authorization", "Bearer sk-gw-top")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .expect("POST /v1/messages via standalone gateway");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resp.headers().get("x-mock-trace").unwrap(),
        "anthropic-messages-json"
    );
    let bytes = resp.bytes().await.expect("read body");
    assert_eq!(&bytes[..], ANTHROPIC_MESSAGES_JSON.as_bytes());

    // mock 收到注入的 key（child 真转发）
    let anth_reqs = anth.requests.lock().await;
    assert!(
        anth_reqs
            .iter()
            .any(|r| recorded_header_get(&r.headers, "x-api-key") == Some("test-anthropic-key")),
        "anthropic mock must receive injected x-api-key"
    );

    // 无 key → 401（独立进程鉴权正常）
    let resp = client
        .post(format!("{base}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4","messages":[]}"#)
        .send()
        .await
        .expect("POST without key");
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // usage 落盘（独立进程同样写 usage.jsonl）
    let records = poll_usage_jsonl(&usage_path, 1).await;
    // 无 per-key 身份：key 恒为空（绝不写 token 本体）。
    assert_eq!(records[0]["key"], "");
    assert_eq!(records[0]["status"], 200);

    let stderr = gw.kill_and_capture_stderr();
    if !stderr.is_empty() {
        eprintln!("sebas gateway stderr:\n{stderr}");
    }
}
