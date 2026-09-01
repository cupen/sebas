//! 文件工具（task 3.3，design N6）：read（行号 + 分页 + 二进制/目录探测）、
//! write / edit（read-before-write 门控、精确匹配计数、tmp+rename 原子写）。

use super::{resolve_under, Tool, ToolCtx};
use crate::message::{ToolErrorKind, ToolOutput};
use std::io::Write as _;
use std::path::{Path, PathBuf};

// ─── read ────────────────────────────────────────────────────────────────────

pub struct ReadTool;

/// read 输出上限（与 bash 同量级；超出尾部截断）。
const READ_OUTPUT_CAP: usize = 30_000;

/// 单文件读取：行号前缀（1 起），offset/limit 以行为单位。
pub(crate) fn read_lines(
    path: &Path,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<String, ToolErrorKind> {
    let meta = std::fs::metadata(path).map_err(|e| ToolErrorKind::Io(format!("metadata {path:?}: {e}")))?;
    if meta.is_dir() {
        return Err(ToolErrorKind::InvalidArgs);
    }
    let bytes = std::fs::read(path).map_err(|e| ToolErrorKind::Io(e.to_string()))?;
    // 二进制探测：前 8KB 内出现 NUL 即按二进制拒读（与 git/claude 约定一致）。
    let probe_len = bytes.len().min(8192);
    if bytes[..probe_len].contains(&0u8) {
        return Err(ToolErrorKind::Denied {
            reason: "binary file".into(),
        });
    }
    let text = String::from_utf8_lossy(&bytes);
    let start = offset.unwrap_or(0).max(1).saturating_sub(1) as usize; // 1-based → 0-based
    let max = limit.map(|l| start + l as usize).unwrap_or(usize::MAX);
    let mut out = String::new();
    for (i, line) in text.lines().enumerate().skip(start).take(max.saturating_sub(start)) {
        out.push_str(&format!("{:>6}\t{}\n", i + 1, line));
    }
    Ok(out)
}

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }

    fn description(&self) -> String {
        "Read a text file with 1-based line numbers. Supports offset/limit paging for \
         large files. Refuses directories and binary files. Read before write or edit."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to the session workdir)."},
                "offset": {"type": "integer", "description": "1-based first line to read."},
                "limit": {"type": "integer", "description": "Max number of lines to read."}
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(p) = input.get("path").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `path`");
        };
        let path = resolve_under(&ctx.workdir, p);
        let offset = input.get("offset").and_then(|v| v.as_u64());
        let limit = input.get("limit").and_then(|v| v.as_u64());
        match read_lines(&path, offset, limit) {
            Ok(text) => {
                ctx.mark_read(&path);
                ToolOutput::ok(text).capped(READ_OUTPUT_CAP)
            }
            Err(kind) => match kind {
                ToolErrorKind::InvalidArgs => ToolOutput::error(
                    ToolErrorKind::InvalidArgs,
                    format!("{p:?} is a directory, not a file"),
                ),
                ToolErrorKind::Denied { reason } => ToolOutput::error(
                    ToolErrorKind::Denied { reason: reason.clone() },
                    format!("cannot read {p:?}: {reason}"),
                ),
                other => ToolOutput::error(other, format!("cannot read {p:?}")),
            },
        }
    }
}

// ─── write / edit ────────────────────────────────────────────────────────────

pub struct WriteTool;
pub struct EditTool;

/// tmp + rename 原子落盘（沿 router/src/state_store.rs 的既有惯例）。
fn atomic_write(path: &Path, content: &str) -> Result<(), ToolErrorKind> {
    let tmp = path.with_extension(format!(
        "{}tmp-{}",
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{e}."))
            .unwrap_or_default(),
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| ToolErrorKind::Io(e.to_string()))?;
        f.write_all(content.as_bytes())
            .map_err(|e| ToolErrorKind::Io(e.to_string()))?;
        f.sync_all().map_err(|e| ToolErrorKind::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| ToolErrorKind::Io(e.to_string()))?;
    Ok(())
}

/// write / edit 共用：read-before-write 门控（spec「Write without prior read is
/// refused」）。新文件豁免（不存在即无从读起）。
fn gate_read_before_modify(ctx: &ToolCtx, path: &Path) -> Result<(), ToolOutput> {
    if path.exists() && !ctx.was_read(path) {
        return Err(ToolOutput::error(
            ToolErrorKind::Denied {
                reason: "read-before-write".into(),
            },
            format!(
                "refused: {path:?} exists but was not read in this session; read it first"
            ),
        ));
    }
    Ok(())
}

fn parse_path(input: &serde_json::Value, ctx: &ToolCtx) -> Result<PathBuf, ToolOutput> {
    let Some(p) = input.get("path").and_then(|v| v.as_str()) else {
        return Err(ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `path`"));
    };
    Ok(resolve_under(&ctx.workdir, p))
}

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }

    fn description(&self) -> String {
        "Create or overwrite a text file atomically. Refuses to overwrite an existing \
         file that was not read earlier in this session."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to the session workdir)."},
                "content": {"type": "string", "description": "Full file content to write."}
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let path = match parse_path(&input, ctx) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let Some(content) = input.get("content").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `content`");
        };
        if let Err(e) = gate_read_before_modify(ctx, &path) {
            return e;
        }
        // 父目录缺失则创建（新文件落在新子目录是合法操作）。
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return ToolOutput::error(ToolErrorKind::Io(e.to_string()), "mkdir parents failed");
        }
        match atomic_write(&path, content) {
            Ok(()) => {
                // 不 mark_read：write 不等价于 read，再次覆盖仍须先读。
                ToolOutput::ok(format!("wrote {} bytes to {}", content.len(), path.display()))
            }
            Err(kind) => ToolOutput::error(kind, format!("write {path:?} failed")),
        }
    }
}

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &'static str {
        "edit"
    }

    fn description(&self) -> String {
        "Replace an exact literal substring in an existing file. The match must be \
         unique unless replace_all is set; the error reports the actual match count. \
         The file must have been read earlier in this session."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to the session workdir)."},
                "old_string": {"type": "string", "description": "Exact literal text to replace."},
                "new_string": {"type": "string", "description": "Replacement text."},
                "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)."}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let path = match parse_path(&input, ctx) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let Some(old) = input.get("old_string").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `old_string`");
        };
        let Some(new) = input.get("new_string").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `new_string`");
        };
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if let Err(e) = gate_read_before_modify(ctx, &path) {
            return e;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::error(ToolErrorKind::NotFound, format!("read failed: {e}"));
            }
        };
        let count = content.matches(old).count();
        if count == 0 {
            return ToolOutput::error(
                ToolErrorKind::InvalidArgs,
                format!("edit refused: 0 matches for the given old_string in {path:?}"),
            );
        }
        if count > 1 && !replace_all {
            // spec「Edit with ambiguous match is refused」：报实际匹配数。
            return ToolOutput::error(
                ToolErrorKind::InvalidArgs,
                format!(
                    "edit refused: old_string matches {count} locations in {path:?}; \
                     make it unique or pass replace_all"
                ),
            );
        }
        let updated = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };
        match atomic_write(&path, &updated) {
            Ok(()) => {
                let replaced = if replace_all { count } else { 1 };
                ToolOutput::ok(format!("edited {path:?}: {replaced} replacement(s)"))
            }
            Err(kind) => ToolOutput::error(kind, format!("edit {path:?} write failed")),
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

    fn tools() -> (ReadTool, WriteTool, EditTool) {
        (ReadTool, WriteTool, EditTool)
    }

    #[tokio::test]
    async fn read_numbers_lines_and_pages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();
        let (read, _, _) = tools();
        let c = ctx(dir.path());

        let full = read
            .execute(serde_json::json!({"path": "f.txt"}), &c)
            .await;
        assert!(full.ok, "{}", full.output);
        assert!(full.output.contains("     1\tone"));
        assert!(full.output.contains("     4\tfour"));

        let paged = read
            .execute(
                serde_json::json!({"path": "f.txt", "offset": 2, "limit": 2}),
                &c,
            )
            .await;
        assert!(paged.output.contains("     2\ttwo"));
        assert!(paged.output.contains("     3\tthree"));
        assert!(!paged.output.contains("one"));
        assert!(!paged.output.contains("four"));
    }

    #[tokio::test]
    async fn read_refuses_directory_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        let (read, _, _) = tools();
        let c = ctx(dir.path());

        let d = read.execute(serde_json::json!({"path": "."}), &c).await;
        assert!(!d.ok);
        assert!(d.output.contains("directory"));

        let bin = dir.path().join("b.bin");
        std::fs::write(&bin, [0u8, 1, 2, 0, 3]).unwrap();
        let b = read.execute(serde_json::json!({"path": "b.bin"}), &c).await;
        assert!(!b.ok);
        assert!(b.output.contains("binary"));
    }

    #[tokio::test]
    async fn write_new_file_ok_but_overwrite_requires_read() {
        let dir = tempfile::tempdir().unwrap();
        let (_, write, _) = tools();
        let c = ctx(dir.path());

        let w1 = write
            .execute(serde_json::json!({"path": "new.txt", "content": "v1"}), &c)
            .await;
        assert!(w1.ok);

        // 文件已存在但本会话没读过 → 拒，文件内容不变。
        let w2 = write
            .execute(serde_json::json!({"path": "new.txt", "content": "v2"}), &c)
            .await;
        assert!(!w2.ok, "overwrite without read must be refused");
        assert!(w2.output.contains("read it first"), "{}", w2.output);
        assert_eq!(std::fs::read_to_string(dir.path().join("new.txt")).unwrap(), "v1");

        // read 之后放行。
        let (read, _, _) = tools();
        let _ = read.execute(serde_json::json!({"path": "new.txt"}), &c).await;
        let w3 = write
            .execute(serde_json::json!({"path": "new.txt", "content": "v3"}), &c)
            .await;
        assert!(w3.ok);
        assert_eq!(std::fs::read_to_string(dir.path().join("new.txt")).unwrap(), "v3");
    }

    #[tokio::test]
    async fn edit_ambiguous_match_reports_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code.rs");
        std::fs::write(&path, "let a = 1;\nlet a = 2;\n").unwrap();
        let (read, _, edit) = tools();
        let c = ctx(dir.path());
        let _ = read.execute(serde_json::json!({"path": "code.rs"}), &c).await;

        // 两处匹配且未开 replace_all → 拒 + 报匹配数，文件不变。
        let e1 = edit
            .execute(
                serde_json::json!({"path": "code.rs", "old_string": "let a =", "new_string": "let b ="}),
                &c,
            )
            .await;
        assert!(!e1.ok);
        assert!(e1.output.contains("2 locations"), "{}", e1.output);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "let a = 1;\nlet a = 2;\n"
        );

        // replace_all 放行，两处都替换。
        let e2 = edit
            .execute(
                serde_json::json!({"path": "code.rs", "old_string": "let a =", "new_string": "let b =", "replace_all": true}),
                &c,
            )
            .await;
        assert!(e2.ok, "{}", e2.output);
        assert!(e2.output.contains("2 replacement(s)"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "let b = 1;\nlet b = 2;\n"
        );

        // 0 匹配也是明确的错误数据。
        let e3 = edit
            .execute(
                serde_json::json!({"path": "code.rs", "old_string": "nope", "new_string": "x"}),
                &c,
            )
            .await;
        assert!(!e3.ok);
        assert!(e3.output.contains("0 matches"));
    }

    #[tokio::test]
    async fn write_is_atomic_no_tmp_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let (_, write, _) = tools();
        let c = ctx(dir.path());
        let _ = write
            .execute(serde_json::json!({"path": "x.txt", "content": "data"}), &c)
            .await;
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "tmp file left behind: {leftovers:?}");
    }
}
