//! 真实 CLI 冒烟（manual）：本机 PATH 装有 npm 版 claude 时，解析器应把它
//! 解析成 `.cmd` 包装且能完成 `--version` 查询——「Failed to get Claude
//! version: program not found」的回归钉（cc-agent-sdk 对无扩展名程序只找
//! `.exe`，见 win_exe 模块文档）。PATH 上没有 claude 的机器诚实跳过。

#[tokio::test]
#[ignore = "real-CLI smoke; run with -- --ignored on a machine that has claude installed"]
async fn resolves_npm_claude_and_reads_version() {
    let resolved = sebas_acp::resolve_windows_executable("claude");
    let out = tokio::process::Command::new(&resolved)
        .arg("--version")
        .output()
        .await
        .unwrap_or_else(|e| panic!("spawn resolved claude ({resolved}) failed: {e}"));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "claude --version failed: {text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.to_lowercase().contains("claude"),
        "version text should name claude: {text}"
    );
}
