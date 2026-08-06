//! `sebas record` 录制链路：以 fake-claude 为被录 agent，驱动新方言
//! （control_request initialize → user message 一轮对话），断言 fixture
//! 文件按 {"dir","msg"} journal 格式记下双向流量。

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
        r#"{"type":"control_request","request_id":"r1","request":{"subtype":"initialize"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#,
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
    assert!(journal.len() >= 5, "in+out lines recorded: {journal:?}");

    // in 方向逐条保留。
    let ins: Vec<_> = journal.iter().filter(|j| j["dir"] == "in").collect();
    assert_eq!(ins.len(), 2);
    assert_eq!(ins[0]["msg"]["request"]["subtype"], "initialize");
    assert_eq!(ins[1]["msg"]["type"], "user");

    // out 方向：initialize 应答、system init、assistant 帧、result 帧。
    let outs: Vec<_> = journal.iter().filter(|j| j["dir"] == "out").collect();
    assert!(outs.len() >= 3, "agent responses recorded: {outs:?}");
    assert!(
        outs.iter().any(|j| j["msg"]["type"] == "control_response"),
        "initialize ack captured: {outs:?}"
    );
    assert!(
        outs.iter().any(|j| j["msg"]["type"] == "assistant"),
        "assistant frames captured: {outs:?}"
    );

    // 顺序：首条必为 in 的 initialize。
    assert_eq!(journal[0]["dir"], "in");
    assert_eq!(journal[0]["msg"]["request"]["subtype"], "initialize");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn non_json_lines_are_forwarded_but_not_recorded() {
    let dir = std::env::temp_dir().join(format!("sebas-record-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("fixture.jsonl");

    // 一行非 JSON 混在中间：fake-claude 跳过它（保真行为），record 也跳过。
    let input = "not json at all\n{\"type\":\"control_request\",\"request_id\":\"r1\",\"request\":{\"subtype\":\"initialize\"}}\n";
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
            .any(|j| j["dir"] == "in" && j["msg"]["request"]["subtype"] == "initialize"),
        "valid line still recorded"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
