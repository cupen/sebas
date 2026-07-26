use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut session_started = false;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let kind = v.get("type").and_then(|k| k.as_str()).unwrap_or("");
        if !session_started && kind == "create_session" {
            session_started = true;
            let sid = v
                .get("session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("s1");
            writeln!(
                stdout,
                "{{\"type\":\"text_delta\",\"session_id\":\"{sid}\",\"delta\":\"hello \"}}"
            )
            .ok();
            writeln!(
                stdout,
                "{{\"type\":\"text_delta\",\"session_id\":\"{sid}\",\"delta\":\"world\"}}"
            )
            .ok();
            writeln!(
                stdout,
                "{{\"type\":\"finished\",\"session_id\":\"{sid}\"}}"
            )
            .ok();
            stdout.flush().ok();
        }
        // Echo other commands; ignore.
    }
}
