//! `sebas record` 录制链路：以 fake-claude 为被录 agent，驱动
//! initialize → session/new → session/prompt 一轮对话，断言 fixture
//! 文件按 fake-claude journal 格式（{"dir","msg"}）记下双向流量。

use sebas::config::Config;
use std::path::PathBuf;

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/fake-claude")
}

fn test_cfg() -> Config {
    let toml = format!(
        r#"
[feishu]
app_id = "cli_x"
app_secret = "sec"

[acp.claude]
path = {:?}
"#,
        fake().to_str().unwrap()
    );
    Config::parse(&toml).unwrap()
}

fn read_journal(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .expect("fixture exists")
        .lines()
        .map(|l| serde_json::from_str(l).expect("journal line is JSON"))
        .collect()
}

#[tokio::test]
async fn records_both_directions_in_journal_format() {
    let dir = std::env::temp_dir().join(format!("sebas-record-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("fixture.jsonl");

    // 用户输入一轮最小对话；EOF（Cursor 自然结束）关闭子进程 stdin →
    // fake-claude 退出 → 录制结束。
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"sess-1","prompt":[{"type":"text","text":"hi"}]}}"#,
        "\n",
    );
    let term: Vec<u8> = Vec::new();

    sebas::record::record_with_io(
        &test_cfg(),
        &[],
        std::io::Cursor::new(input.as_bytes().to_vec()),
        term,
        &out,
    )
    .await
    .expect("record completes on child exit");

    let journal = read_journal(&out);
    assert!(journal.len() >= 6, "in+out lines recorded: {journal:?}");

    // in 方向逐条保留。
    let ins: Vec<_> = journal.iter().filter(|j| j["dir"] == "in").collect();
    assert_eq!(ins.len(), 3);
    assert_eq!(ins[0]["msg"]["method"], "initialize");
    assert_eq!(ins[2]["msg"]["method"], "session/prompt");

    // out 方向：initialize 响应、session/new 响应、update、prompt 响应。
    let outs: Vec<_> = journal.iter().filter(|j| j["dir"] == "out").collect();
    assert!(outs.len() >= 3, "agent responses recorded: {outs:?}");
    assert!(
        outs.iter()
            .any(|j| j["msg"]["result"]["sessionId"] == "sess-1"),
        "session/new response captured: {outs:?}"
    );
    assert!(
        outs.iter().any(|j| j["msg"]["method"] == "session/update"),
        "streaming update captured: {outs:?}"
    );

    // 顺序：首条必为 in/initialize。
    assert_eq!(journal[0]["dir"], "in");
    assert_eq!(journal[0]["msg"]["method"], "initialize");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn non_json_lines_are_forwarded_but_not_recorded() {
    let dir = std::env::temp_dir().join(format!("sebas-record-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("fixture.jsonl");

    // 一行非 JSON 混在中间：fake-claude 跳过它（保真行为），record 也跳过。
    let input = "not json at all\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1}}\n";
    let term: Vec<u8> = Vec::new();

    sebas::record::record_with_io(
        &test_cfg(),
        &[],
        std::io::Cursor::new(input.as_bytes().to_vec()),
        term,
        &out,
    )
    .await
    .expect("record completes");

    let journal = read_journal(&out);
    assert!(
        journal.iter().all(|j| j["msg"].is_object()),
        "no garbage lines in fixture"
    );
    assert!(
        journal
            .iter()
            .any(|j| j["dir"] == "in" && j["msg"]["method"] == "initialize"),
        "valid line still recorded"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
