#[derive(Debug, PartialEq)]
pub enum Command {
    New,
    Sessions,
    Switch(usize),
    Resume(String),
    Cancel,
    Status,
    Compact,
    Cost,
    Model(String),
    Cd(String),
    Help,
    Btw(String),
    /// `/settings` | `/settings <key>` | `/settings <key> <value>`.
    Settings(Option<String>, Option<String>),
    PassThrough(String),
}

pub fn parse_command(input: &str) -> Command {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        return Command::PassThrough(format!("/{rest}"));
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();
    match head {
        "/new" => Command::New,
        "/sessions" => Command::Sessions,
        "/switch" => match arg.parse::<usize>() {
            Ok(n) => Command::Switch(n),
            Err(_) => Command::PassThrough(input.into()),
        },
        "/resume" => Command::Resume(arg.into()),
        "/cancel" => Command::Cancel,
        "/status" => Command::Status,
        "/compact" => Command::Compact,
        "/cost" => Command::Cost,
        "/model" => Command::Model(arg.into()),
        "/cd" => Command::Cd(arg.into()),
        "/help" => Command::Help,
        "/settings" => {
            let mut kv = arg.splitn(2, char::is_whitespace);
            let key = kv.next().unwrap_or("").trim();
            let val = kv.next().unwrap_or("").trim();
            let key = if key.is_empty() { None } else { Some(key.to_string()) };
            let val = if val.is_empty() { None } else { Some(val.to_string()) };
            Command::Settings(key, val)
        }
        "/btw" => {
            if arg.is_empty() {
                Command::PassThrough(input.into())
            } else {
                Command::Btw(arg.into())
            }
        }
        _ => Command::PassThrough(input.into()),
    }
}
