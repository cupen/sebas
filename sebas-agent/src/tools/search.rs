//! 搜索工具（task 3.4，design N6）：glob（walkdir + globset，mtime 序，
//! 100 条即停，跳过 .git/target）与 grep（regex 逐行匹配，按文件分组，
//! 250 条即停；不依赖 rg 二进制）。

use super::{is_ignored_dir_name, resolve_under, Tool, ToolCtx};
use crate::message::{ToolErrorKind, ToolOutput};
use std::path::{Path, PathBuf};

/// glob 结果上限（spec：Glob over 100 files stops at the cap）。
pub const GLOB_CAP: usize = 100;
/// grep 匹配上限（spec：Grep over 250 matches stops at the cap）。
pub const GREP_CAP: usize = 250;

/// 收集 `root` 下的普通文件（跳过 .git/target/node_modules），按 mtime 降序
/// （新文件在前——与 fake-claude 仓库的 glob 语义一致）。
fn collect_files(root: &Path, include: Option<&globset::Glob>) -> Vec<PathBuf> {
    let matcher = include.map(|g| g.compile_matcher());
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // 目录剪枝：根本身不剪，只剪被忽略的子目录。
            e.depth() == 0 || !is_ignored_dir_name(e.file_name().to_string_lossy().as_ref())
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Some(m) = &matcher {
            let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
            if !m.is_match(rel) && !m.is_match(entry.path()) {
                continue;
            }
        }
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);
        files.push((mtime, entry.path().to_path_buf()));
    }
    // mtime 新→旧；同刻用路径稳定排序。
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    files.into_iter().map(|(_, p)| p).collect()
}

pub struct GlobTool;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn description(&self) -> String {
        "Find files by glob pattern (e.g. \"src/**/*.rs\"), newest first. Stops at 100 \
         matches and marks the result truncated. Skips .git, target, node_modules."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern relative to the session workdir."},
                "path": {"type": "string", "description": "Optional subdirectory to search (default: workdir)."}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `pattern`");
        };
        let root = match input.get("path").and_then(|v| v.as_str()) {
            Some(sub) => resolve_under(&ctx.workdir, sub),
            None => ctx.workdir.clone(),
        };
        if !root.is_dir() {
            return ToolOutput::error(
                ToolErrorKind::NotFound,
                format!("search root {} is not a directory", root.display()),
            );
        }
        // globset 的 Glob 要求字面分隔符语义：`**/*.rs` 直接可用。
        let glob = match globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
        {
            Ok(g) => g,
            Err(e) => {
                return ToolOutput::error(
                    ToolErrorKind::InvalidArgs,
                    format!("bad glob pattern {pattern:?}: {e}"),
                );
            }
        };
        let files = collect_files(&root, Some(&glob));
        let truncated = files.len() > GLOB_CAP;
        let shown: Vec<String> = files
            .iter()
            .take(GLOB_CAP)
            .map(|p| {
                p.strip_prefix(&ctx.workdir)
                    .unwrap_or(p)
                    .display()
                    .to_string()
            })
            .collect();
        let mut out = if shown.is_empty() {
            format!("no matches for {pattern:?}")
        } else {
            shown.join("\n")
        };
        if truncated {
            out.push_str(&format!(
                "\n[truncated: {} more files beyond the {}-file cap]",
                files.len() - GLOB_CAP,
                GLOB_CAP
            ));
        }
        ToolOutput {
            ok: true,
            output: out,
            truncated,
            exit_code: None,
            error: None,
        }
    }
}

pub struct GrepTool;

/// 单文件 grep：逐行正则匹配，返回 (行号, 行文本) 对，达 cap 即停。
fn grep_file(path: &Path, re: &regex::Regex, cap: usize) -> std::io::Result<Vec<(usize, String)>> {
    let bytes = std::fs::read(path)?;
    // 二进制（NUL 探测）与非法 UTF-8 文件跳过（lossy 会污染行号语义的精确性）。
    let probe_len = bytes.len().min(8192);
    if bytes[..probe_len].contains(&0u8) {
        return Ok(Vec::new());
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(Vec::new());
    };
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if re.is_match(line) {
            hits.push((i + 1, line.to_string()));
            if hits.len() >= cap {
                break;
            }
        }
    }
    Ok(hits)
}

#[async_trait::async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn description(&self) -> String {
        "Search file contents with a regular expression, grouped per file with line \
         numbers. Stops at 250 matches and marks the result truncated. Filters by an \
         optional include glob (e.g. \"*.rs\"). No external binary needed."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regular expression (Rust regex syntax)."},
                "include": {"type": "string", "description": "Filename glob filter, e.g. \"*.ts\"."},
                "path": {"type": "string", "description": "Optional subdirectory to search (default: workdir)."}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `pattern`");
        };
        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::error(
                    ToolErrorKind::InvalidArgs,
                    format!("bad regex {pattern:?}: {e}"),
                );
            }
        };
        let include = match input.get("include").and_then(|v| v.as_str()) {
            Some(inc) => match globset::GlobBuilder::new(inc)
                .literal_separator(false)
                .build()
            {
                Ok(g) => Some(g.compile_matcher()),
                Err(e) => {
                    return ToolOutput::error(
                        ToolErrorKind::InvalidArgs,
                        format!("bad include glob {inc:?}: {e}"),
                    );
                }
            },
            None => None,
        };
        let root = match input.get("path").and_then(|v| v.as_str()) {
            Some(sub) => resolve_under(&ctx.workdir, sub),
            None => ctx.workdir.clone(),
        };
        if !root.is_dir() {
            return ToolOutput::error(
                ToolErrorKind::NotFound,
                format!("search root {} is not a directory", root.display()),
            );
        }

        // 收集全部候选文件（include 过滤），再逐文件 grep 到达 cap 即停。
        let files = collect_files(&root, None);
        let mut total = 0usize;
        let mut truncated = false;
        let mut groups: Vec<String> = Vec::new();
        for f in &files {
            if total >= GREP_CAP {
                truncated = true;
                break;
            }
            if let Some(m) = &include {
                let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if !m.is_match(&name) {
                    continue;
                }
            }
            let remaining = GREP_CAP - total;
            let hits = match grep_file(f, &re, remaining) {
                Ok(h) => h,
                Err(_) => continue, // 不可读文件 warn-and-skip
            };
            if hits.is_empty() {
                continue;
            }
            total += hits.len();
            if hits.len() == remaining && hits.len() == GREP_CAP {
                truncated = true; // 正好打满 cap
            }
            let rel = f.strip_prefix(&ctx.workdir).unwrap_or(f).display();
            let body: Vec<String> = hits
                .iter()
                .map(|(n, line)| format!("{n}:{line}"))
                .collect();
            groups.push(format!("{}:\n{}", rel, body.join("\n")));
        }

        let mut out = if groups.is_empty() {
            format!("no matches for {pattern:?}")
        } else {
            groups.join("\n")
        };
        if truncated {
            out.push_str(&format!(
                "\n[truncated: stopped at the {}-match cap]",
                GREP_CAP
            ));
        }
        ToolOutput {
            ok: true,
            output: out,
            truncated,
            exit_code: None,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(workdir: &std::path::Path) -> ToolCtx {
        ToolCtx::new(
            workdir.to_path_buf(),
            tokio_util::sync::CancellationToken::new(),
        )
    }

    /// 造一棵小树：src/a.rs、src/deep/b.rs、node_modules/skip.rs、.git/skip2.rs。
    fn tree(dir: &Path) {
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("src/deep/b.rs"), "fn helper() {}\n").unwrap();
        std::fs::write(dir.join("node_modules/skip.rs"), "fn noise() {}\n").unwrap();
        std::fs::write(dir.join(".git/skip2.rs"), "fn noise() {}\n").unwrap();
        // mtime 顺序：b.rs 最新。
        let a = dir.join("src/a.rs");
        let b = dir.join("src/deep/b.rs");
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let f = std::fs::File::options().append(true).open(&a).unwrap();
        f.set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t)).unwrap();
        let _ = (a, b);
    }

    #[tokio::test]
    async fn glob_finds_files_skips_ignored_dirs() {
        let dir = tempfile::tempdir().unwrap();
        tree(dir.path());
        let out = GlobTool
            .execute(
                serde_json::json!({"pattern": "**/*.rs"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("src/a.rs"));
        assert!(out.output.contains("src/deep/b.rs"));
        assert!(!out.output.contains("node_modules"), "{}", out.output);
        assert!(!out.output.contains(".git/"));
        assert!(!out.truncated);
    }

    #[tokio::test]
    async fn glob_cap_reports_truncation() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(GLOB_CAP + 20) {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let out = GlobTool
            .execute(serde_json::json!({"pattern": "*.txt"}), &ctx(dir.path()))
            .await;
        assert!(out.truncated, "must stop at the cap with a truncation flag");
        assert!(out.output.contains("[truncated"));
        assert_eq!(out.output.lines().filter(|l| l.contains(".txt") && !l.contains("[truncated")).count(), GLOB_CAP);
    }

    #[tokio::test]
    async fn grep_groups_per_file_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/one.rs"), "alpha\nbeta alpha\n").unwrap();
        std::fs::write(dir.path().join("src/two.rs"), "gamma\n").unwrap();
        let out = GrepTool
            .execute(
                serde_json::json!({"pattern": "alpha", "include": "*.rs"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("src/one.rs:"));
        assert!(out.output.contains("1:alpha"));
        assert!(out.output.contains("2:beta alpha"));
        assert!(!out.output.contains("two.rs"), "only matching files appear");
    }

    #[tokio::test]
    async fn grep_cap_and_binary_skip() {
        let dir = tempfile::tempdir().unwrap();
        // 250 + 10 个匹配文件。
        for i in 0..(GREP_CAP + 10) {
            std::fs::write(dir.path().join(format!("m{i}.txt")), "needle\n").unwrap();
        }
        // 二进制文件（NUL）即使内容含 needle 也被跳过。
        std::fs::write(dir.path().join("bin.dat"), [0u8, b'n', b'e', 0u8]).unwrap();
        let out = GrepTool
            .execute(
                serde_json::json!({"pattern": "needle", "include": "*"}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.truncated);
        assert!(out.output.contains("250-match cap"));
        assert!(!out.output.contains("bin.dat"));

        // grep_file 的 cap 语义：单文件内 250 行即停。
        let f = dir.path().join("one-big.txt");
        std::fs::write(&f, "needle\n".repeat(400)).unwrap();
        let re = regex::Regex::new("needle").unwrap();
        let hits = grep_file(&f, &re, GREP_CAP).unwrap();
        assert_eq!(hits.len(), GREP_CAP);
    }

    #[tokio::test]
    async fn bad_pattern_is_invalid_args() {
        let dir = tempfile::tempdir().unwrap();
        let out = GrepTool
            .execute(serde_json::json!({"pattern": "(["}), &ctx(dir.path()))
            .await;
        assert!(!out.ok);
        assert!(matches!(out.error, Some(ToolErrorKind::InvalidArgs)));

        let out2 = GlobTool
            .execute(serde_json::json!({"pattern": "["}), &ctx(dir.path()))
            .await;
        assert!(!out2.ok);
    }
}
