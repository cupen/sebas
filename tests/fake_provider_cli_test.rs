//! `sebas fake-provider` CLI 动词的进程级冒烟（fake-provider-upstream 1.2/2.1）。
//!
//! 与 sandbox 配方无关：fake 上游不读任何 sebas 配置/env，只写调用方指定的
//! journal 路径（本测试给 TestDir 内的路径）。覆盖三件事：
//!
//! - `--help` 正常（动词接线）；
//! - 坏 scenario = 启动失败：退出码 75、stderr 末行 `startup-failure: …`；
//! - 正常起服务：stdout ready 行可解析 → 拨 `/v1/messages` 得 200 → SIGTERM
//!   优雅退出（退出码 0）且端口释放。
//!
//! 非 `#[ignore]`：秒级，进 `cargo test --workspace` 门禁。

mod support;

use std::process::Stdio;
use std::time::Duration;

use support::TestDir;

/// 有界轮询 ready 行，返回解析出的 `SocketAddr`。
async fn wait_ready(log: &std::path::Path) -> std::net::SocketAddr {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(text) = std::fs::read_to_string(log) {
            for line in text.lines().rev() {
                if let Some(idx) = line.find("fake-provider listening addr=") {
                    let addr = line[idx + "fake-provider listening addr=".len()..]
                        .split_whitespace()
                        .next()
                        .unwrap_or_default();
                    if let Ok(addr) = addr.parse::<std::net::SocketAddr>() {
                        return addr;
                    }
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "fake-provider never printed a parseable ready line; log at {}",
            log.display()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn fake_provider_help_is_wired() {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
        .args(["fake-provider", "--help"])
        .output()
        .await
        .expect("spawn --help");
    assert!(out.status.success(), "--help must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in ["--listen", "--scenario", "--journal"] {
        assert!(
            stdout.contains(flag),
            "--help must document {flag}: {stdout}"
        );
    }
}

#[tokio::test]
async fn fake_provider_bad_scenario_exits_75_with_summary() {
    let dir = TestDir::new("fake_provider_cli", "bad-scenario");
    let scenario = dir.path().join("bad.json");
    std::fs::write(&scenario, "{ not json").expect("write bad scenario");

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
        .args([
            "fake-provider",
            "--listen",
            "127.0.0.1:0",
            "--scenario",
            &scenario.to_string_lossy(),
        ])
        .output()
        .await
        .expect("spawn fake-provider");
    assert_eq!(
        out.status.code(),
        Some(75),
        "scenario load failure = EX_TEMPFAIL; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr.lines().filter(|l| !l.trim().is_empty()).next_back();
    assert!(
        last.is_some_and(|l| l.starts_with("startup-failure: ")),
        "stderr last line must carry the summary: {stderr:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn fake_provider_serves_and_exits_on_sigterm() {
    use std::os::unix::process::ExitStatusExt;

    let dir = TestDir::new("fake_provider_cli", "serve");
    let log = dir.path().join("fake.log");
    let journal = dir.path().join("journal.jsonl");
    let log_file = std::fs::File::create(&log).expect("create log");
    let log_err = log_file.try_clone().expect("clone log handle");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
        .args([
            "fake-provider",
            "--listen",
            "127.0.0.1:0",
            "--journal",
            &journal.to_string_lossy(),
        ])
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn fake-provider");

    let addr = wait_ready(&log).await;
    assert!(addr.ip().is_loopback(), "fake upstream must bind loopback");

    let cli = reqwest::Client::new();
    let (status, body) = {
        let resp = cli
            .post(format!("http://{addr}/v1/messages"))
            .header("content-type", "application/json")
            .header("x-api-key", "sk-downstream-must-not-leak")
            .body(r#"{"model":"fake-model","messages":[{"role":"user","content":"hi"}]}"#)
            .send()
            .await
            .expect("dial fake provider");
        let status = resp.status().as_u16();
        let body = resp.text().await.expect("body");
        (status, body)
    };
    assert_eq!(status, 200, "POST /v1/messages: {body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["stop_reason"], "end_turn");
    assert!(v["content"][0]["text"].is_string());

    // journal 落盘（留痕契约）。
    let journal_text = std::fs::read_to_string(&journal).expect("journal written");
    let line: serde_json::Value =
        serde_json::from_str(journal_text.lines().next().expect("one line")).expect("json line");
    assert_eq!(line["method"], "POST");
    assert_eq!(line["path"], "/v1/messages");
    assert_eq!(line["headers"]["x-api-key"], "sk-downstream-must-not-leak");

    // SIGTERM：优雅退出（信号致死 = 未处理，判失败）且端口释放。
    unsafe {
        libc::kill(child.id().expect("pid") as libc::pid_t, libc::SIGTERM);
    }
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("fake-provider must exit on SIGTERM")
        .expect("wait");
    assert_eq!(
        status.signal(),
        None,
        "SIGTERM must be handled gracefully, not terminate the process"
    );
    assert_eq!(status.code(), Some(0), "graceful exit code");
    tokio::net::TcpListener::bind(addr)
        .await
        .expect("port must be released after exit");
}

/// 非 unix：没有 SIGTERM；用 kill 拆卸并只断言端口释放。
#[cfg(not(unix))]
#[tokio::test]
async fn fake_provider_serves_and_releases_port_on_kill() {
    let dir = TestDir::new("fake_provider_cli", "serve");
    let log = dir.path().join("fake.log");
    let log_file = std::fs::File::create(&log).expect("create log");
    let log_err = log_file.try_clone().expect("clone log handle");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
        .args(["fake-provider", "--listen", "127.0.0.1:0"])
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true)
        .spawn()
        .expect("spawn fake-provider");
    let addr = wait_ready(&log).await;
    assert!(
        reqwest::Client::new()
            .post(format!("http://{addr}/v1/messages"))
            .body("{}")
            .send()
            .await
            .is_ok_and(|r| r.status().as_u16() == 200)
    );
    child.kill().await.expect("kill");
    tokio::net::TcpListener::bind(addr)
        .await
        .expect("port released");
}
