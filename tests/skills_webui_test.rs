//! `/api/skills*` handler 级验收（add-agent-skills 5.1）：真文件系统服务
//! （`sebas::skills::FsSkillsService`，与 CLI 同一份 core）注入 webui
//! router，覆盖四端点的 happy path、name 不存在 404 与目录穿越拒绝。
//! 安全约定：store / backend 落点全部在 tempdir 沙箱，绝不写真实 HOME。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sebas::skills::FsSkillsService;
use sebas_feishu::cards::CardConfig;
use sebas_webui::agent_kinds::ConfigAgentKindProvider;
use sebas_webui::auth::AuthHandle;
use sebas_webui::build_router_with_skills;
use sebas_webui::models::RouterInfo;
use sebas_webui::session_backend::FakeBackend;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const SKILL_BODY: &str = "---\nname: beads\ndescription: beads 工作流\n---\n\n# beads\n";

/// 沙箱 + 接好真服务的 router：store = `<root>/skills`，claude 落点 =
/// `<root>/home/.claude/skills`，gemini 进 no_placement。
struct Harness {
    _root: tempfile::TempDir,
    app: axum::Router,
    store: PathBuf,
    backend: PathBuf,
}

fn make_skill(root: &Path, name: &str, body: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), body).unwrap();
    dir
}

impl Harness {
    fn new() -> Harness {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("skills");
        let backend = root.path().join("home").join(".claude").join("skills");
        let service = FsSkillsService::with_placements(
            store.clone(),
            vec![("claude".into(), backend.clone())],
            vec!["gemini".into()],
        );
        let app = build_router_with_skills(
            Arc::new(FakeBackend::new()),
            RouterInfo::default(),
            CardConfig::default(),
            Arc::new(ConfigAgentKindProvider::new(Vec::new())),
            Arc::new(AuthHandle::disabled()),
            root.path().to_path_buf(),
            Arc::new(service),
        );
        Harness {
            _root: root,
            app,
            store,
            backend,
        }
    }

    async fn req(&self, method: &str, uri: &str, body: Option<String>) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:12345");
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let req = builder.body(Body::from(body.unwrap_or_default())).unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, v)
    }
}

// ── GET /api/skills ──────────────────────────────────────────────────────────

#[tokio::test]
async fn list_reports_entries_with_attachments_and_invalid_reason() {
    let h = Harness::new();
    make_skill(&h.store, "beads", SKILL_BODY);
    let deploy = make_skill(
        &h.store,
        "my-deploy",
        "---\nname: my-deploy\ndescription: 部署\n---\nbody",
    );
    std::fs::create_dir_all(deploy.join("scripts")).unwrap();
    std::fs::write(deploy.join("scripts").join("deploy.sh"), "#!/bin/sh").unwrap();
    make_skill(&h.store, "broken", "没有 SKILL.md 的目录");

    let (status, body) = h.req("GET", "/api/skills", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let skills = body["skills"].as_array().expect("skills array");
    assert_eq!(skills.len(), 3);

    let beads = skills.iter().find(|s| s["name"] == "beads").unwrap();
    assert_eq!(beads["description"], "beads 工作流");
    assert_eq!(beads["valid"], true);
    assert!(beads["attachments"].as_array().unwrap().is_empty());
    assert!(
        beads.get("reason").is_none(),
        "valid 条目不带 reason: {beads}"
    );

    let deploy = skills.iter().find(|s| s["name"] == "my-deploy").unwrap();
    assert_eq!(
        deploy["attachments"][0], "scripts/deploy.sh",
        "attachment 是 `/` 分隔的相对路径"
    );

    let broken = skills.iter().find(|s| s["name"] == "broken").unwrap();
    assert_eq!(broken["valid"], false);
    assert!(
        broken["reason"].as_str().unwrap().contains("SKILL.md"),
        "invalid 附原因（spec：honestly surfaced）: {broken}"
    );
}

/// 仓目录缺失 = 空仓：200 + 空数组，不是错误。
#[tokio::test]
async fn list_missing_store_is_empty_array_ok() {
    let h = Harness::new();
    let (status, body) = h.req("GET", "/api/skills", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["skills"].as_array().unwrap().is_empty(), "{body}");
}

// ── GET /api/skills/{name} ───────────────────────────────────────────────────

#[tokio::test]
async fn detail_returns_skill_md_text_and_attachments() {
    let h = Harness::new();
    let deploy = make_skill(
        &h.store,
        "my-deploy",
        "---\nname: my-deploy\ndescription: 部署\n---\n# 正文",
    );
    std::fs::write(deploy.join("ref.md"), "doc").unwrap();

    let (status, body) = h.req("GET", "/api/skills/my-deploy", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "my-deploy");
    assert_eq!(
        body["text"], SKILL_MD_BODY,
        "text 是 SKILL.md 原文（不渲染）"
    );
    assert_eq!(body["attachments"][0], "ref.md");
}

const SKILL_MD_BODY: &str = "---\nname: my-deploy\ndescription: 部署\n---\n# 正文";

#[tokio::test]
async fn detail_unknown_name_is_404() {
    let h = Harness::new();
    let (status, body) = h.req("GET", "/api/skills/definitely-absent", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("definitely-absent"),
        "{body}"
    );
}

/// 目录穿越拒绝：`/`、`\`、`..` 一律 400（先安全后存在性），仓外文件无恙。
#[tokio::test]
async fn detail_rejects_path_traversal_names() {
    let h = Harness::new();
    // 仓外「机密」：若穿越成立，detail 会读到它。
    let escape = h._root.path().join("escape");
    std::fs::create_dir_all(&escape).unwrap();
    std::fs::write(escape.join("SKILL.md"), "TOP SECRET").unwrap();

    for name in ["..%2Fescape", "a%2Fb", "..%5Cescape", ".", ".."] {
        let uri = format!("/api/skills/{name}");
        let (status, body) = h.req("GET", &uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        let msg = body["error"].as_str().unwrap_or_default();
        assert!(
            !msg.contains("TOP SECRET"),
            "错误信息不得泄漏仓外内容: {msg}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(escape.join("SKILL.md")).unwrap(),
        "TOP SECRET",
        "仓外文件安然无恙"
    );
}

// ── DELETE /api/skills/{name} ────────────────────────────────────────────────

#[tokio::test]
async fn delete_removes_store_entry_and_leaves_backend_untouched() {
    let h = Harness::new();
    make_skill(&h.store, "beads", SKILL_BODY);
    // 先投影一次，backend 里已有副本。
    let (status, body) = h.req("POST", "/api/skills/sync", Some("{}".into())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(h.backend.join("beads").is_dir(), "前置：投影已发生");

    let (status, body) = h.req("DELETE", "/api/skills/beads", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "deleted");
    assert!(!h.store.join("beads").exists(), "仓内条目被删");
    assert!(
        h.backend.join("beads").is_dir(),
        "DELETE 只删仓：backend 副本原样（清理归下一次 sync）"
    );

    // 删除后再 sync：投影随仓删除（镜像语义闭环）。
    let (status, body) = h.req("POST", "/api/skills/sync", Some("{}".into())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reports"][0]["deleted"][0], "beads", "{body}");
    assert!(!h.backend.join("beads").exists());
}

#[tokio::test]
async fn delete_unknown_name_is_404_and_traversal_is_400() {
    let h = Harness::new();
    let (status, body) = h.req("DELETE", "/api/skills/definitely-absent", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, _) = h.req("DELETE", "/api/skills/..%2Fescape", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "穿越名 400");
}

// ── POST /api/skills/sync ────────────────────────────────────────────────────

#[tokio::test]
async fn sync_writes_placements_and_reports_no_placement() {
    let h = Harness::new();
    make_skill(&h.store, "beads", SKILL_BODY);

    let (status, body) = h.req("POST", "/api/skills/sync", Some("{}".into())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reports"][0]["backend"], "claude");
    assert_eq!(body["reports"][0]["written"], serde_json::json!(["beads"]));
    assert_eq!(
        body["no_placement"],
        serde_json::json!(["gemini"]),
        "无落点 backend 如实报告，不许静默跳过（spec）"
    );
    assert_eq!(
        std::fs::read_to_string(h.backend.join("beads").join("SKILL.md")).unwrap(),
        SKILL_BODY
    );

    // 覆盖语义：「仓 wins」+ overwritten 如实呈现。（改写必须保持有效
    // frontmatter——invalid 条目不投影，是 core 的既有语义。）
    std::fs::write(
        h.store.join("beads").join("SKILL.md"),
        "---\nname: beads\ndescription: 新版\n---\nnew version",
    )
    .unwrap();
    let (_, body) = h.req("POST", "/api/skills/sync", Some("{}".into())).await;
    assert_eq!(
        body["reports"][0]["overwritten"],
        serde_json::json!(["beads"]),
        "{body}"
    );
    assert!(
        std::fs::read_to_string(h.backend.join("beads").join("SKILL.md"))
            .unwrap()
            .ends_with("new version")
    );
}

/// invalid 条目的详情（add-agent-skills 5.1 wire 语义）：条目在仓但缺
/// SKILL.md → 200 + `text=null`，不冒充 404（core `skill_detail` 的「诚实
/// 呈现」经 handler 落到 wire）。
#[tokio::test]
async fn detail_of_invalid_entry_is_200_with_null_text_not_404() {
    let h = Harness::new();
    // 目录在、SKILL.md 文件真缺失（invalid 的「缺文件」成因）。
    let broken = h.store.join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("ref.md"), "doc").unwrap();

    let (status, body) = h.req("GET", "/api/skills/broken", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "broken");
    assert!(
        body["text"].is_null(),
        "缺 SKILL.md 的条目 text=null（诚实呈现）: {body}"
    );
    assert_eq!(body["attachments"], serde_json::json!(["ref.md"]));
}
