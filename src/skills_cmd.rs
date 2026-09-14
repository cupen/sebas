//! `sebas skills` — 操作者级 skill 仓的命令行面（add-agent-skills 4.1–4.4）。
//!
//! 薄壳（design D4）：扫仓 / 三源落仓 / 只删仓 / 全量投影的实体全在
//! [`crate::skills`]，本模块只做参数解析、三源分派与输出排版。输出约定：
//!
//! ```text
//! $ sebas skills list
//! beads  beads 工作流
//! my-deploy  部署脚本 (2 attachments)
//! broken  [invalid] 缺少 SKILL.md
//!
//! $ sebas skills sync
//! claude: written=1 overwritten=0 deleted=0 private_ignored=2 (<dir>)
//!   + beads
//! gemini: no placement（无 skill 目录约定，未写任何文件）
//! ```
//!
//! 仓目录缺失按空仓处理（列表打一行说明，退出码 0，不是错误）。

use crate::config::Config;
use crate::error::{Result, SebasError};
use crate::skills::{self, BackendOutcome, SkillInfo};
use std::path::{Path, PathBuf};

/// `sebas skills` 的参数（`-c` 与其他子命令同一惯例）。
pub struct SkillsArgs {
    pub config: String,
    pub cmd: SkillsCmd,
}

/// `sebas skills` 子命令。
pub enum SkillsCmd {
    /// 列出仓内条目（name / description / invalid 标记附原因）。
    List,
    /// 落仓一个来源：本地目录 / git URL / 其余交 `npx skills add`。
    Add { source: String },
    /// 只删仓内条目（绝不动 backend；清理归下一次 sync）。
    Remove { name: String },
    /// 全量投影到 configured backends，输出逐 backend 汇总。
    Sync,
}

/// CLI 入口：读配置 → 取仓目录 → 分派。一次性管理命令（非服务），失败按
/// 普通错误退出 1。
pub fn run(args: SkillsArgs) -> Result<()> {
    let raw = std::fs::read_to_string(&args.config)
        .map_err(|e| SebasError::Config(format!("read config {}: {e}", args.config)))?;
    let cfg = Config::parse(&raw)?;
    let store = PathBuf::from(cfg.skills_dir());

    match args.cmd {
        SkillsCmd::List => {
            for line in list_lines(&store) {
                println!("{line}");
            }
        }
        SkillsCmd::Add { source } => add(&store, &source)?,
        SkillsCmd::Remove { name } => {
            skills::remove_skill(&store, &name)?;
            println!("removed {name}（只删了仓；backend 里的副本将在下次 sync 时清理）");
        }
        SkillsCmd::Sync => {
            let home = skills::resolve_home();
            let kinds = skills::configured_kinds(&cfg);
            let outcome = skills::sync_all(&store, &kinds, &home)?;
            for line in sync_lines(&outcome) {
                println!("{line}");
            }
        }
    }
    Ok(())
}

/// add 的三源分派结果（design D4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddSource {
    /// 本地目录（存在且是目录 → 校验 SKILL.md 后整目录拷贝）。
    Local(PathBuf),
    /// git URL（http(s) / git@ 前缀，或 .git 后缀）。
    Git(String),
    /// 其余一切 → `npx skills add <pkg>`。
    Npx(String),
}

/// 三源分派（tasks 4.2）：已是本地目录的最优先（URL 不可能是本地目录；
/// 恰好叫 `x.git` 的本地目录按本地处理——它在磁盘上，用户的意图更直接）；
/// 其次 git 形态判定；其余全部交 npx。
pub fn classify_source(src: &str) -> AddSource {
    if Path::new(src).is_dir() {
        return AddSource::Local(PathBuf::from(src));
    }
    let looks_git = src.starts_with("http://")
        || src.starts_with("https://")
        || src.starts_with("git@")
        || src.ends_with(".git");
    if looks_git {
        AddSource::Git(src.to_string())
    } else {
        AddSource::Npx(src.to_string())
    }
}

fn add(store: &Path, source: &str) -> Result<()> {
    match classify_source(source) {
        AddSource::Local(path) => {
            let name = skills::add_from_local(&path, store)?;
            println!("added {name} → {}", store.join(&name).display());
        }
        AddSource::Git(url) => {
            let names = skills::add_from_git(&url, store)?;
            for name in &names {
                println!("added {name} → {}", store.join(name).display());
            }
        }
        AddSource::Npx(pkg) => {
            skills::add_from_npx(&pkg, store)?;
            println!(
                "npx skills add {pkg} 完成（落点由 skills CLI 决定，不经 sebas 仓）"
            );
        }
    }
    Ok(())
}

/// `skills list` 的输出行（导出供测试断言排版）：`name  description
/// (N attachments)`；invalid 条目 `name  [invalid] 原因`；空仓（目录缺失
/// 同）打一行说明、不报错。
pub fn list_lines(store: &Path) -> Vec<String> {
    let items = skills::scan_store(store);
    if items.is_empty() {
        return vec![format!("（skill 仓为空：{}）", store.display())];
    }
    items.iter().map(format_skill_line).collect()
}

fn format_skill_line(s: &SkillInfo) -> String {
    match (&s.invalid_reason, s.description.as_deref()) {
        (Some(reason), _) => format!("{}  [invalid] {}", s.name, reason),
        (None, Some(description)) if s.attachments.is_empty() => {
            format!("{}  {}", s.name, description)
        }
        (None, Some(description)) => format!(
            "{}  {} ({} attachments)",
            s.name,
            description,
            s.attachments.len()
        ),
        // valid 恒带 description；这行只为穷尽性兜底。
        (None, None) => s.name.clone(),
    }
}

/// `skills sync` 的输出行（导出供测试断言排版）：逐 backend 一行汇总
/// `{written, overwritten, deleted, private_ignored}` + 落点目录；非空列表
/// 逐名带 `+`（写）/`~`（覆盖）/`-`（删）前缀；无落点 backend 如实一行
/// no placement（spec「reported, not skipped」）。
pub fn sync_lines(outcome: &[BackendOutcome]) -> Vec<String> {
    let mut lines = Vec::new();
    for o in outcome {
        match &o.report {
            Some(report) => {
                lines.push(format!(
                    "{}: written={} overwritten={} deleted={} private_ignored={}",
                    o.backend,
                    report.written.len(),
                    report.overwritten.len(),
                    report.deleted.len(),
                    report.private_ignored
                ));
                for name in &report.written {
                    lines.push(format!("  + {name}"));
                }
                for name in &report.overwritten {
                    lines.push(format!("  ~ {name}（仓 wins，已覆盖）"));
                }
                for name in &report.deleted {
                    lines.push(format!("  - {name}（仓里已删，投影随删）"));
                }
            }
            None => lines.push(format!(
                "{}: no placement（无 skill 目录约定，未写任何文件）",
                o.backend
            )),
        }
    }
    lines
}
