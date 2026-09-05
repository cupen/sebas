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
/model <text> — 透传 /model 指令给 claude code\n\
/goal <text> — 透传 /goal 指令给 claude code\n\
/cd <path> — 切换工作目录\n\
/settings [key [value]] — 查看/修改卡片配置\n\
/upgrade [dev|--dev] [dry-run|--dry-run] — 通过 watchdog 升级并重启\n\
/rollback — 通过 watchdog 回滚并重启\n\
/restart — 通过 watchdog 重启 core\n\
/confirm <token> — 确认执行待确认的危险操作（/upgrade /rollback /restart 提交后）\n\
/services — 查看 watchdog 服务状态\n\
/system — 查看 watchdog 系统状态\n\
/router on|off|restart|status — 管理 router 服务\n\
/webui status — 查看 webui 服务状态\n\
/btw <text> — 插队提问\n\
/help — 显示本帮助\n\
（注：watchdog 控制命令需 watchdog 在线且核心已配置控制凭据）";

/// `/router` 的动作域（openspec/specs/router-commands/spec.md control commands）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterAction {
    On,
    Off,
    Restart,
    Status,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    /// `/new [prompt]` —— 开新会话，trailing text 作为初始 prompt：
    /// `derive_topic(prompt)` 派生卡片标题、引用块渲染 prompt。空 trailing
    /// 退化为旧行为（无 prompt、卡标题回退 `"Claude Code"` 占位）。
    New(String),
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
    Cd(String),
    Help,
    Btw(String),
    /// `/settings` | `/settings <key>` | `/settings <key> <value>`.
    Settings(Option<String>, Option<String>),
    Upgrade {
        dev: bool,
        dry_run: bool,
    },
    Rollback,
    Restart,
    /// `/confirm <token>` — 兑换待确认危险操作的令牌（sebas-29s）。
    /// token 为空时入站层回复用法提示。
    Confirm(String),
    Services,
    /// `/system` — watchdog 系统状态（openspec/specs/router-commands/spec.md control commands）。
    System,
    /// `/router on|off|restart|status` — 管理 router 服务（openspec/specs/router-commands/spec.md）。
    Router(RouterAction),
    /// `/webui status` — 查看 webui 服务状态（openspec/specs/router-commands/spec.md）。
    Webui,
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
        "/new" => Command::New(arg.into()),
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
        "/model" | "/goal" => Command::PassThrough(input.into()),
        "/cd" => Command::Cd(arg.into()),
        "/help" => Command::Help,
        "/settings" => {
            let mut kv = arg.splitn(2, char::is_whitespace);
            let key = kv.next().unwrap_or("").trim();
            let val = kv.next().unwrap_or("").trim();
            let key = if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            };
            let val = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            };
            Command::Settings(key, val)
        }
        "/upgrade" => {
            let args: Vec<_> = arg.split_whitespace().collect();
            let normalized: Vec<&str> = args
                .iter()
                .map(|a| if *a == "dev" { "--dev" } else { *a })
                .collect();
            if normalized
                .iter()
                .all(|a| matches!(*a, "--dev" | "--dry-run"))
            {
                Command::Upgrade {
                    dev: normalized.contains(&"--dev"),
                    dry_run: normalized.contains(&"--dry-run"),
                }
            } else {
                Command::PassThrough(input.into())
            }
        }
        "/rollback" if arg.is_empty() => Command::Rollback,
        "/restart" if arg.is_empty() => Command::Restart,
        // `/confirm <token>`：token 为空也解析为 Confirm（入站层回用法提示），
        // 不走 PassThrough——把裸 /confirm 转发给 claude code 毫无意义。
        "/confirm" => Command::Confirm(arg.into()),
        "/services" if arg.is_empty() => Command::Services,
        // `/system` 是只读状态命令，与 `/status` 同级：忽略尾随参数（例如
        // `/system  now` 仍视为 System），不要求空 arg。
        "/system" => Command::System,
        // `/gateway` 是改名前的命令词（rename-cli-surface）：保留为别名。
        "/router" | "/gateway" => match arg {
            "on" => Command::Router(RouterAction::On),
            "off" => Command::Router(RouterAction::Off),
            "restart" => Command::Router(RouterAction::Restart),
            "status" => Command::Router(RouterAction::Status),
            _ => Command::PassThrough(input.into()),
        },
        "/webui" if arg == "status" => Command::Webui,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_system_command() {
        assert_eq!(parse_command("/system"), Command::System);
        assert_eq!(parse_command("/system  extra"), Command::System);
        assert_eq!(
            parse_command("/systemx"),
            Command::PassThrough("/systemx".into())
        );
    }

    #[test]
    fn parses_router_actions() {
        assert_eq!(
            parse_command("/router on"),
            Command::Router(RouterAction::On)
        );
        assert_eq!(
            parse_command("/router off"),
            Command::Router(RouterAction::Off)
        );
        assert_eq!(
            parse_command("/router restart"),
            Command::Router(RouterAction::Restart)
        );
        assert_eq!(
            parse_command("/router status"),
            Command::Router(RouterAction::Status)
        );
        // invalid action falls through to passthrough
        assert_eq!(
            parse_command("/router foobar"),
            Command::PassThrough("/router foobar".into())
        );
        // bare /router with no action
        assert_eq!(
            parse_command("/router"),
            Command::PassThrough("/router".into())
        );
    }

    #[test]
    fn parses_confirm_command() {
        assert_eq!(
            parse_command("/confirm tok_abc123"),
            Command::Confirm("tok_abc123".into())
        );
        // 裸 /confirm 不走 PassThrough（转发给 claude code 毫无意义），
        // 解析为空 token 由入站层回用法提示。
        assert_eq!(parse_command("/confirm"), Command::Confirm(String::new()));
        assert_eq!(
            parse_command("/confirmtx"),
            Command::PassThrough("/confirmtx".into())
        );
    }

    #[test]
    fn parses_webui_status() {
        assert_eq!(parse_command("/webui status"), Command::Webui);
        // anything else for /webui falls through
        assert_eq!(
            parse_command("/webui on"),
            Command::PassThrough("/webui on".into())
        );
    }
}
