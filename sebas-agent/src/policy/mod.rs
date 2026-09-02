//! 权限策略（task 1.1–1.3，design N1）：三层判定 + 交互审批 + fail-closed。
//!
//! 判定顺序：静态 deny > 会话精确签名 allowlist > 静态 allow > 内置默认分类
//! （只读放行 / 破坏面 ask / 网络面按 NetworkMode）。ask 无回答者 → 拒绝
//! （fail-closed，DH-2）；拒绝可经审批人一次性升级重试（带理由，仅放行一次）。

pub mod sandbox;

pub use sandbox::{SandboxMode, SandboxProfile};

use crate::tools::resolve_under;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 静态规则：工具名 + 可选参数子集匹配（input 中这些键存在且值相等）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRule {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_subset: Option<Value>,
}

impl ToolRule {
    pub fn tool(tool: &str) -> Self {
        Self {
            tool: tool.to_string(),
            args_subset: None,
        }
    }
}

/// 网络工具总开关（design N3）：off（默认，工具不注册/直接拒）| ask（审批放行）| on（静默）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    Off,
    Ask,
    On,
}

/// 策略配置（静态层 + 网络开关 + 审批超时）。
#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub deny: Vec<ToolRule>,
    pub allow: Vec<ToolRule>,
    pub network: NetworkMode,
    /// ask 等待应答的超时；超时视为无应答 → fail closed。
    pub approval_timeout: Duration,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            deny: Vec::new(),
            allow: Vec::new(),
            network: NetworkMode::Off,
            approval_timeout: Duration::from_secs(300),
        }
    }
}

/// 单次工具调用的策略判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    /// 需要交互审批；执行器经 [`Approver`] 请求决定。
    Ask { reason: String },
}

/// 权限请求的描述（发给回答者/审查卡）。
#[derive(Debug, Clone)]
pub struct PermissionRequestInfo {
    /// 与工具 `tool_use_id` 一致（permission-flow 的关联契约）。
    pub request_id: String,
    pub tool: String,
    pub input: Value,
    pub reason: String,
}

/// 审批应答。`Escalate` = 带理由的一次性放行（DSH-2 升级形态：仅放行那一次，会话策略不变）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAnswer {
    AllowOnce,
    AllowSession,
    Deny,
    Escalate { reason: String },
}

/// 审批回答者（webui 审查卡 / agent-dev 脚本应答 / 测试桩）。
/// 返回 `None` = 无回答者或未应答 → fail closed。
#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, req: &PermissionRequestInfo) -> Option<ApprovalAnswer>;

    /// 在 `PermissionRequest` 事件发出**之前**登记待决请求（消除「应答先于登记」
    /// 的竞态窗口）。默认无操作。
    fn prepare(&self, _request_id: &str) {}

    /// 投递一个审批决定（design N5：`SessionHandle::answer_permission` 转发至此）。
    /// 返回 false = 没有该 request_id 的待决请求。
    fn answer(&self, _request_id: &str, _answer: ApprovalAnswer) -> bool {
        false
    }
}

/// 标准审批枢纽：`prepare()` 在事件发出前登记 oneshot；`approve()` 停靠等待；
/// `answer()` 投递决定。宿主（webui 审查卡 / agent-dev `--answer`）把事件里的
/// `request_id` 原样回传即可。crate 内部用的就是它；自定义回答者直接实现
/// [`Approver`]。
#[derive(Default)]
pub struct ApproverHub {
    senders: Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<ApprovalAnswer>>>,
    receivers: Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Receiver<ApprovalAnswer>>>,
}

impl ApproverHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait::async_trait]
impl Approver for ApproverHub {
    fn prepare(&self, request_id: &str) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.senders
            .lock()
            .expect("approver hub poisoned")
            .insert(request_id.to_string(), tx);
        self.receivers
            .lock()
            .expect("approver hub poisoned")
            .insert(request_id.to_string(), rx);
    }

    async fn approve(&self, req: &PermissionRequestInfo) -> Option<ApprovalAnswer> {
        // prepare 已登记 → 取出对应 receiver；未登记（自定义流程）→ 现建并登记。
        let rx = self
            .receivers
            .lock()
            .expect("approver hub poisoned")
            .remove(&req.request_id)
            .unwrap_or_else(|| {
                let (tx, rx) = tokio::sync::oneshot::channel();
                self.senders
                    .lock()
                    .expect("approver hub poisoned")
                    .insert(req.request_id.clone(), tx);
                rx
            });
        // 发送端消失（会话结束/决定改道）→ None（fail closed）
        rx.await.ok()
    }

    fn answer(&self, request_id: &str, answer: ApprovalAnswer) -> bool {
        match self
            .senders
            .lock()
            .expect("approver hub poisoned")
            .remove(request_id)
        {
            Some(tx) => {
                let _ = tx.send(answer);
                true
            }
            None => false,
        }
    }
}

/// 内置风险分类（默认策略的依据，design N1 表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RiskClass {
    ReadOnly,
    Destructive,
    Network,
}

/// 三层策略引擎。会话 allowlist 为 `(tool, args)` 精确签名集合
/// （serde_json Value 默认 BTreeMap → 序列化键序确定）。
pub struct PolicyEngine {
    config: PolicyConfig,
    session_allow: Mutex<HashSet<String>>,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self {
            config,
            session_allow: Mutex::new(HashSet::new()),
        }
    }

    pub fn config(&self) -> &PolicyConfig {
        &self.config
    }

    /// 会话级精确签名 allowlist：`allow-once` 批准后由执行器调用（allow-session 语义）。
    pub fn allow_session(&self, tool: &str, input: &Value) {
        self.session_allow
            .lock()
            .expect("session allowlist poisoned")
            .insert(Self::signature(tool, input));
    }

    /// 规则命中后的放行也走 allowlist（AllowSession 应答 = 精确签名吸收后续重复调用）。
    pub(crate) fn session_contains(&self, tool: &str, input: &Value) -> bool {
        self.session_allow
            .lock()
            .expect("session allowlist poisoned")
            .contains(&Self::signature(tool, input))
    }

    fn signature(tool: &str, input: &Value) -> String {
        format!("{tool}\u{1f}{}", serde_json::to_string(input).unwrap_or_default())
    }

    fn rule_hit(rules: &[ToolRule], tool: &str, input: &Value) -> bool {
        rules.iter().any(|r| {
            r.tool == tool
                && r.args_subset.as_ref().is_none_or(|sub| match sub {
                    Value::Object(map) => map
                        .iter()
                        .all(|(k, v)| input.get(k) == Some(v)),
                    _ => true,
                })
        })
    }

    /// 单次调用判定（task 1.1）。`workdir` 用于 write/edit 的新文件豁免判定。
    pub fn evaluate(&self, tool: &str, input: &Value, workdir: &Path) -> PolicyDecision {
        // 1. 静态 deny 最高优先（幂等拒绝：deny 永远不进 ask）。
        if Self::rule_hit(&self.config.deny, tool, input) {
            return PolicyDecision::Deny {
                reason: format!("static deny rule matches `{tool}`"),
            };
        }
        // 2. 会话精确签名 allowlist。
        if self.session_contains(tool, input) {
            return PolicyDecision::Allow;
        }
        // 3. 静态 allow。
        if Self::rule_hit(&self.config.allow, tool, input) {
            return PolicyDecision::Allow;
        }
        // 4. 内置默认分类。
        match self.classify(tool, input, workdir) {
            RiskClass::ReadOnly => PolicyDecision::Allow,
            RiskClass::Network => match self.config.network {
                NetworkMode::On => PolicyDecision::Allow,
                NetworkMode::Ask => PolicyDecision::Ask {
                    reason: format!("network tool `{tool}` needs approval"),
                },
                NetworkMode::Off => PolicyDecision::Deny {
                    reason: "network tools are disabled (network = \"off\")".into(),
                },
            },
            RiskClass::Destructive => PolicyDecision::Ask {
                reason: format!("`{tool}` may modify state outside read-only scope"),
            },
        }
    }

    fn classify(&self, tool: &str, input: &Value, workdir: &Path) -> RiskClass {
        match tool {
            "read" | "glob" | "grep" | "read_image" | "lsp" => RiskClass::ReadOnly,
            // 写工具：新文件静默（无从覆盖既有状态），覆盖既有文件 → 破坏面。
            "write" | "edit" => {
                let exists = input
                    .get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| resolve_under(workdir, p).exists())
                    .unwrap_or(true); // 拿不准 → 按覆盖处理
                if exists {
                    RiskClass::Destructive
                } else {
                    RiskClass::ReadOnly
                }
            }
            // bash：疑似只读（探测类命令）静默；其余破坏面。真实边界由 N2 沙箱强制。
            "bash" => {
                let readonly = input
                    .get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(bash_probably_readonly);
                if readonly {
                    RiskClass::ReadOnly
                } else {
                    RiskClass::Destructive
                }
            }
            "web_search" | "web_fetch" => RiskClass::Network,
            // 未知工具保守处理。
            _ => RiskClass::Destructive,
        }
    }
}

// ─── bash 只读启发式（design N1「只读 bash 静默」）────────────────────────────
//
// 保守判定：任何拿不准的情况都返回 false（→ ask）。它只决定"要不要弹卡"，
// 真实的写/网络边界由沙箱后端在内核层强制——误判只影响体验，不影响安全。

const READONLY_HEADS: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "rg", "find", "stat", "file", "du", "df", "wc", "pwd",
    "echo", "printf", "which", "command", "type", "readlink", "basename", "dirname", "realpath",
    "date", "uname", "whoami", "id", "hostname", "printenv", "sort", "uniq", "cut", "awk",
    "sed", "jq", "diff", "cmp", "tree", "ps", "true", "false", "test", "seq", "od", "md5sum",
    "sha1sum", "sha256sum", "cksum", "tac", "rev", "column", "paste", "join", "comm", "iconv",
    "man", "tldr", "help",
];
const GIT_READONLY_SUBS: &[&str] = &[
    "status", "log", "diff", "show", "branch", "tag", "remote", "rev-parse", "describe",
    "ls-files", "ls-remote", "blame", "shortlog", "reflog", "cat-file", "config", "help",
    "version", "grep",
];
const CARGO_READONLY_SUBS: &[&str] = &["check", "test", "tree", "metadata", "locate-project", "read-manifest", "rustc", "--version", "yank"];
/// 作为段首 token 出现即视为破坏面（无论参数）。
const DANGEROUS_HEADS: &[&str] = &[
    "mkfs", "mkfs.ext2", "mkfs.ext3", "mkfs.ext4", "mkfs.vfat", "dd", "shutdown", "reboot",
    "poweroff", "halt", "fdisk", "sfdisk", "wipefs", "sgdisk", "parted", "sudo", "doas", "su",
];
/// 全命令字面子串（fork 炸弹 / 整盘覆盖 / 设备直写）。
const DANGEROUS_PATTERNS: &[&str] = &["rm -rf /", "rm -fr /", ":(){", "of=/dev/", ">/dev/sd", "> /dev/sd", ">>/dev/sd"];

/// 判定 `command` 是否"疑似只读"。按 `; | && ||` 分段，每段首 token 必须在只读
/// 集合内，且段内不得出现写重定向 / 命令替换 / 写倾向 token。
pub fn bash_probably_readonly(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() || DANGEROUS_PATTERNS.iter().any(|p| cmd.contains(p)) {
        return false;
    }
    // 全命令级扫描：写重定向 / 命令替换。`>` 仅在 `>&`（fd 复制，如 2>&1）
    // 或 `>/dev/null` 时放行；`>>` 一律拒绝。必须发生在分段前——分段不能拆
    // 在 '&' 上，否则 2>&1 会被拆坏。
    let bytes = cmd.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'>' {
            continue;
        }
        // '>' 后跳过空白再判断：'&'（fd 复制）或 '/dev/null' 放行，其余拒绝。
        let mut j = i + 1;
        while bytes.get(j).is_some_and(|b| b.is_ascii_whitespace()) {
            j += 1;
        }
        match bytes.get(j).copied() {
            Some(b'&') => {}
            Some(_) if cmd[j..].starts_with("/dev/null") => {}
            _ => return false,
        }
    }
    if cmd.contains(">>") || cmd.contains('`') || cmd.contains("$(") {
        return false;
    }
    // 分段：'&&' 预替换为 ';'；单 '&'（后台）保留在段内不拆。
    cmd.replace("&&", ";")
        .split([';', '|', '\n'])
        .map(str::trim)
        .filter(|seg| !seg.is_empty())
        .all(segment_readonly)
}

fn segment_readonly(seg: &str) -> bool {
    let mut tokens = seg.split_whitespace().peekable();
    let mut head = String::new();
    // 跳过前置环境赋值（FOO=bar cmd → cmd）
    loop {
        match tokens.peek() {
            Some(t) if t.contains('=') && !t.starts_with('-') => head = t.to_string(),
            Some(t) => {
                head = t.to_string();
                tokens.next();
                break;
            }
            None => break,
        }
        tokens.next();
    }
    let head = head.as_str();
    // 取路径基名（/usr/bin/git → git）
    let base = head.rsplit('/').next().unwrap_or(head);
    if DANGEROUS_HEADS.contains(&base) {
        return false;
    }
    let rest: Vec<&str> = tokens.collect();
    let sub = rest.first().copied().unwrap_or("");
    let head_ok = if base == "git" {
        GIT_READONLY_SUBS.contains(&sub)
    } else if base == "cargo" {
        CARGO_READONLY_SUBS.contains(&sub)
    } else if base == "sed" {
        // sed -i 会写文件
        !rest.iter().any(|t| *t == "-i" || t.starts_with("--in-place"))
    } else {
        READONLY_HEADS.contains(&base)
    };
    if !head_ok {
        return false;
    }
    // 段内其余 token 不得是写倾向命令（echo git 之类过度保守 → ask，可接受）。
    const WRITE_TOKENS: &[&str] = &[
        "rm", "rmdir", "mv", "cp", "dd", "chmod", "chown", "chgrp", "kill", "pkill", "killall",
        "tee", "truncate", "shred", "ln", "mkdir", "touch", "rsync", "scp", "curl", "wget",
        "nc", "ssh", "apt", "apt-get", "pacman", "dnf", "yum", "brew", "npm", "pnpm", "yarn",
        "pip", "pip3", "systemctl", "mount", "umount", "crontab", "useradd", "usermod",
        "groupadd", "passwd",
    ];
    !rest
        .iter()
        .any(|t| WRITE_TOKENS.contains(&t.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine(config: PolicyConfig) -> PolicyEngine {
        PolicyEngine::new(config)
    }

    #[test]
    fn static_rules_layer_priority_deny_beats_allow() {
        let e = engine(PolicyConfig {
            deny: vec![ToolRule::tool("bash")],
            allow: vec![ToolRule::tool("bash")],
            ..Default::default()
        });
        assert!(matches!(
            e.evaluate("bash", &json!({"command": "ls"}), Path::new("/tmp")),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn static_allow_silences_destructive_tool() {
        let e = engine(PolicyConfig {
            allow: vec![ToolRule::tool("write")],
            ..Default::default()
        });
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        assert_eq!(
            e.evaluate(
                "write",
                &json!({"path": "f.txt", "content": "y"}),
                dir.path()
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn args_subset_rule_matches_only_equal_values() {
        let e = engine(PolicyConfig {
            allow: vec![ToolRule {
                tool: "bash".into(),
                args_subset: Some(json!({"command": "echo hi"})),
            }],
            ..Default::default()
        });
        assert_eq!(
            e.evaluate("bash", &json!({"command": "echo hi"}), Path::new("/tmp")),
            PolicyDecision::Allow
        );
        // 不同参数不再命中该 allow 规则；`rm` 是破坏面 → Ask（echo 类只读本就放行）
        assert!(matches!(
            e.evaluate("bash", &json!({"command": "rm -rf build"}), Path::new("/tmp")),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn session_allowlist_absorbs_exact_signature_only() {
        let e = engine(PolicyConfig::default());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x").unwrap();
        let input = json!({"path": "f.txt", "content": "y"});
        assert!(matches!(
            e.evaluate("write", &input, dir.path()),
            PolicyDecision::Ask { .. }
        ));
        e.allow_session("write", &input);
        assert_eq!(e.evaluate("write", &input, dir.path()), PolicyDecision::Allow);
        // 不同参数仍要审批
        assert!(matches!(
            e.evaluate("write", &json!({"path": "f.txt", "content": "z"}), dir.path()),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn readonly_tools_allow_new_file_write_is_silent_overwrite_asks() {
        let e = engine(PolicyConfig::default());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            e.evaluate("read", &json!({"path": "a"}), dir.path()),
            PolicyDecision::Allow
        );
        assert_eq!(
            e.evaluate("glob", &json!({"pattern": "*"}), dir.path()),
            PolicyDecision::Allow
        );
        // 新文件静默
        assert_eq!(
            e.evaluate("write", &json!({"path": "new.txt", "content": "x"}), dir.path()),
            PolicyDecision::Allow
        );
        // 覆盖既有文件 → ask
        std::fs::write(dir.path().join("old.txt"), "x").unwrap();
        assert!(matches!(
            e.evaluate("write", &json!({"path": "old.txt", "content": "y"}), dir.path()),
            PolicyDecision::Ask { .. }
        ));
        // edit 同理
        assert!(matches!(
            e.evaluate("edit", &json!({"path": "old.txt", "old_string": "x", "new_string": "y"}), dir.path()),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn network_mode_gates_web_tools() {
        let dir = Path::new("/tmp");
        let off = engine(PolicyConfig::default());
        assert!(matches!(
            off.evaluate("web_fetch", &json!({"url": "https://x"}), dir),
            PolicyDecision::Deny { .. }
        ));
        let ask = engine(PolicyConfig {
            network: NetworkMode::Ask,
            ..Default::default()
        });
        assert!(matches!(
            ask.evaluate("web_search", &json!({"query": "x"}), dir),
            PolicyDecision::Ask { .. }
        ));
        let on = engine(PolicyConfig {
            network: NetworkMode::On,
            ..Default::default()
        });
        assert_eq!(
            on.evaluate("web_search", &json!({"query": "x"}), dir),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn unknown_tool_is_conservatively_destructive() {
        let e = engine(PolicyConfig::default());
        assert!(matches!(
            e.evaluate("mystery", &json!({}), Path::new("/tmp")),
            PolicyDecision::Ask { .. }
        ));
    }

    #[test]
    fn bash_readonly_heuristic() {
        // 只读：探测类命令
        for ok in [
            "ls -la",
            "cat foo.txt | grep bar",
            "git status && git log --oneline -3",
            "cargo check 2>&1 | head -50",
            "echo hello > /dev/null",
            "cat a 2>&1",
            "pwd; date; whoami",
            "find . -name '*.rs' | wc -l",
            "FOO=1 printenv",
            "sed -n '1,10p' file.txt",
        ] {
            assert!(bash_probably_readonly(ok), "should be readonly: {ok}");
        }
        // 非只读：写 / 网络 / 拿不准
        for bad in [
            "rm -rf build",
            "touch new.txt",
            "echo hi > out.txt",
            "cat a >> b",
            "sed -i 's/a/b/' f.txt",
            "git push origin main",
            "cargo install foo",
            "curl https://example.com",
            "echo $(whoami > /tmp/x)",
            "sudo rm x",
            "dd if=/dev/zero of=/dev/sda",
            "mkdir -p a/b",
            "",
            "$(curl evil)",
            "ls; rm x",
        ] {
            assert!(!bash_probably_readonly(bad), "should NOT be readonly: {bad}");
        }
    }
}
