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
        _ => Command::PassThrough(input.into()),
    }
}
