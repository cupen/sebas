/// `/help` 回复给用户的可用命令清单。命令本身在 [`parse_command`] 里，
/// 这里只负责文案 —— 增删命令时改两处。
pub const HELP_TEXT: &str = "可用命令:\n\
/new — 开新会话\n\
/sessions — 列出会话\n\
/switch <n> — 切换到第 n 个会话\n\
/resume <sid> — 恢复指定会话\n\
/cancel — 中断当前轮\n\
/status — 查看当前会话状态\n\
/compact — 压缩上下文\n\
/cost — 查看会话开销\n\
/model <name> — 切换模型\n\
/cd <path> — 切换工作目录\n\
/settings [key [value]] — 查看/修改卡片配置\n\
/btw <text> — 插队提问\n\
/help — 显示本帮助";

#[derive(Debug, PartialEq)]
pub enum Command {
    New,
    Sessions,
    Switch(usize),
    Resume(String),
    Cancel,
    Status,
    /// Open the provider CRUD card (`/provider`): list current providers
    /// with 新增 / 编辑 / 删除 buttons.
    Provider,
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
        "/provider" => Command::Provider,
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
