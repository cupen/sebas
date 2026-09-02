//! 多模态与语言工具（task 4.2，design N6）：能力门诚实声明。
//!
//! - `read_image`：仅在配置声明模型具备图像输入时注册（spec：text-only 模型
//!   不可见该工具）。本阶段返回图像元数据 + base64；真正的 tool_result 图像
//!   块传输属 Phase 3（消息模型升级）。
//! - `lsp`：语言服务后端可达才上报 `file_system` 能力；不可达时返回
//!   unavailable（非 error——"没有语言服务"不是故障，是事实）。

use super::{resolve_under, Tool, ToolCtx};
use crate::message::{ToolErrorKind, ToolOutput};

// ─── read_image ──────────────────────────────────────────────────────────────

pub struct ReadImageTool;

/// 魔数探测：PNG / JPEG / GIF / WebP。
fn sniff_image_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// 极简 base64（标准字母表 + padding）。
pub(crate) fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[async_trait::async_trait]
impl Tool for ReadImageTool {
    fn name(&self) -> &'static str {
        "read_image"
    }

    fn description(&self) -> String {
        "Read a local image file (PNG/JPEG/GIF/WebP) and return its metadata and \
         base64 payload. Only available when the configured model supports image input."
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Image file path (relative to the session workdir)."}
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(p) = input.get("path").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `path`");
        };
        let path = resolve_under(&ctx.workdir, p);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return ToolOutput::error(ToolErrorKind::NotFound, format!("read {p:?}: {e}")),
        };
        let Some(media_type) = sniff_image_format(&bytes) else {
            return ToolOutput::error(
                ToolErrorKind::InvalidArgs,
                format!("{p:?} is not a recognized image (PNG/JPEG/GIF/WebP)"),
            );
        };
        // base64 直接进输出文本：Phase 2 的诚实边界——模型可见元数据与数据，
        // 真正的 tool_result 图像块传输待消息模型升级（Phase 3）。
        let b64 = base64_encode(&bytes);
        ToolOutput::ok(format!(
            "image {p:?}: {media_type}, {} bytes\nbase64:\n{b64}",
            bytes.len()
        ))
    }
}

// ─── lsp ─────────────────────────────────────────────────────────────────────

/// 语言服务后端（宿主注入；crate 不实现协议本身）。
#[async_trait::async_trait]
pub trait LspBackend: Send + Sync {
    /// operation: definitions | references | hover；返回给模型看的文本。
    async fn query(&self, operation: &str, path: &str, line: u32, column: u32) -> ToolOutput;
}

pub struct LspTool {
    backend: Option<std::sync::Arc<dyn LspBackend>>,
}

impl LspTool {
    pub fn new(backend: Option<std::sync::Arc<dyn LspBackend>>) -> Self {
        Self { backend }
    }
}

#[async_trait::async_trait]
impl Tool for LspTool {
    fn name(&self) -> &'static str {
        "lsp"
    }

    fn description(&self) -> String {
        // file_system 能力字段：仅在后端可达时如实上报（spec 4.2）。
        match self.backend {
            Some(_) => "Code intelligence via the language server: definitions, \
                        references, hover. file_system: available."
                .into(),
            None => "Code intelligence via the language server. Currently unavailable: \
                     no language server is attached to this session (reported as a fact, \
                     not an error)."
                .into(),
        }
    }

    fn parameters(&self) -> serde_json::Value {
        let mut props = serde_json::json!({
            "operation": {"type": "string", "enum": ["definitions", "references", "hover"]},
            "path": {"type": "string"},
            "line": {"type": "integer"},
            "column": {"type": "integer"}
        });
        if self.backend.is_some() {
            // file_system 能力字段（spec：仅在服务器可达时上报）
            props["file_system"] = serde_json::json!(true);
        }
        serde_json::json!({
            "type": "object",
            "properties": props,
            "required": ["operation", "path", "line", "column"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolCtx) -> ToolOutput {
        let Some(backend) = &self.backend else {
            // unavailable 是事实陈述，不是错误（spec：not error）。
            return ToolOutput::ok("unavailable: no language server attached to this session");
        };
        let Some(op) = input.get("operation").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `operation`");
        };
        let Some(path) = input.get("path").and_then(|v| v.as_str()) else {
            return ToolOutput::error(ToolErrorKind::InvalidArgs, "missing `path`");
        };
        let line = input.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let column = input.get("column").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        backend.query(op, path, line, column).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn ctx(dir: &std::path::Path) -> ToolCtx {
        ToolCtx::new(dir.to_path_buf(), CancellationToken::new())
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[tokio::test]
    async fn read_image_sniffs_format_and_encodes() {
        let dir = tempfile::tempdir().unwrap();
        let png: Vec<u8> = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3].to_vec();
        std::fs::write(dir.path().join("a.png"), &png).unwrap();
        let out = ReadImageTool
            .execute(serde_json::json!({"path": "a.png"}), &ctx(dir.path()))
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("image/png"));
        assert!(out.output.contains(&base64_encode(&png)));

        // 非图像 → InvalidArgs
        std::fs::write(dir.path().join("t.txt"), b"hello").unwrap();
        let out = ReadImageTool
            .execute(serde_json::json!({"path": "t.txt"}), &ctx(dir.path()))
            .await;
        assert!(!out.ok);
        assert!(matches!(out.error, Some(ToolErrorKind::InvalidArgs)));
    }

    struct FakeLsp;
    #[async_trait::async_trait]
    impl LspBackend for FakeLsp {
        async fn query(&self, op: &str, path: &str, line: u32, _col: u32) -> ToolOutput {
            ToolOutput::ok(format!("{op} at {path}:{line}"))
        }
    }

    #[tokio::test]
    async fn lsp_without_backend_is_unavailable_fact_not_error() {
        let tool = LspTool::new(None);
        // description 不上报 file_system
        assert!(!tool.description().contains("file_system: available"));
        assert!(!tool.parameters()["properties"].get("file_system").is_some());
        // 调用 → ok:true 的 unavailable 事实
        let dir = tempfile::tempdir().unwrap();
        let out = tool
            .execute(
                serde_json::json!({"operation": "definitions", "path": "a.rs", "line": 1, "column": 2}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok, "unavailable is a fact, not an error: {}", out.output);
        assert!(out.output.contains("unavailable"));
    }

    #[tokio::test]
    async fn lsp_with_backend_reports_file_system_and_queries() {
        let tool = LspTool::new(Some(std::sync::Arc::new(FakeLsp)));
        assert!(tool.description().contains("file_system: available"));
        assert_eq!(tool.parameters()["properties"]["file_system"], serde_json::json!(true));
        let dir = tempfile::tempdir().unwrap();
        let out = tool
            .execute(
                serde_json::json!({"operation": "hover", "path": "a.rs", "line": 3, "column": 4}),
                &ctx(dir.path()),
            )
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.contains("hover at a.rs:3"));
    }

    #[test]
    fn image_block_round_trips_on_anthropic_wire() {
        let block = crate::message::ContentBlock::Image {
            source: crate::message::ImageSource::Base64 {
                media_type: "image/png".into(),
                data: "aGVsbG8=".into(),
            },
        };
        let j = serde_json::to_value(&block).unwrap();
        assert_eq!(j["type"], "image");
        assert_eq!(j["source"]["type"], "base64");
        assert_eq!(j["source"]["media_type"], "image/png");
        let back: crate::message::ContentBlock = serde_json::from_value(j).unwrap();
        assert_eq!(back, block);
    }
}
