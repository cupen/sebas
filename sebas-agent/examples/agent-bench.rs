//! agent-bench 冒烟 CLI（task 6.1，agent-bench spec）：sebas-agent 能力评估面。
//!
//! 用法：
//! ```text
//! cargo run -p sebas-agent --example agent-bench -- --smoke
//! cargo run -p sebas-agent --example agent-bench -- --tasks error_recovery,static_processing
//! cargo run -p sebas-agent --example agent-bench -- --record /tmp/bench.jsonl --debug
//! cargo run -p sebas-agent --example agent-bench -- --replay
//! ```
//!
//! 客户端固定为脚本化 FakeLlmClient（确定性断言的前提）；`--model` 仅进
//! 环境上报。真客户端评测属后续（LlmClient 换实现即可）。将来 `sebas
//! agent-bench` 子命令直接调用 `sebas_agent::bench::run`。

use sebas_agent::bench;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mut smoke = false;
    let mut debug = false;
    let mut replay = false;
    let mut record: Option<std::path::PathBuf> = None;
    let mut _model = "bench-fake-1".to_string();
    let mut tasks_filter: Vec<String> = Vec::new();

    while let Some(a) = args.next() {
        match a.as_str() {
            "--smoke" => smoke = true,
            "--debug" => debug = true,
            "--replay" => replay = true,
            "--record" => {
                record = Some(std::path::PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--record needs a value");
                    std::process::exit(2);
                })));
            }
            "--tasks" => {
                let v = args.next().unwrap_or_else(|| {
                    eprintln!("--tasks needs a value");
                    std::process::exit(2);
                });
                tasks_filter = v.split(',').map(str::to_string).collect();
            }
            "--model" => {
                _model = args.next().unwrap_or_else(|| {
                    eprintln!("--model needs a value");
                    std::process::exit(2);
                });
            }
            "--help" | "-h" => {
                eprintln!(
                    "usage: agent-bench [--smoke] [--tasks a,b,c] [--model m] \
                     [--record FILE] [--debug] [--replay]"
                );
                return;
            }
            other => {
                eprintln!("unknown flag {other:?} (--help for usage)");
                std::process::exit(2);
            }
        }
    }

    let run = bench::run(smoke, &tasks_filter, record.as_deref(), debug, replay).await;
    print!("{}", run.dashboard());

    let failed = run.results.iter().filter(|r| !r.passed).count();
    if failed > 0 {
        std::process::exit(1);
    }
}
