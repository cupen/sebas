//! spec-diff 覆盖门禁（Task 10 / sebas-lva.10）。
//!
//! 官方端点清单 vs 覆盖矩阵的双向断言：
//! - **断言一（反向防自欺）**：`COVERED` 每个端点必须真实存在于 spec 清单。
//!   防止把拼写错误或已废弃的路径写进 COVERED 后"绿"了却没测任何真东西。
//! - **断言二（显式 pending）**：每个 spec 端点要么在 COVERED，要么命中某条
//!   PENDING 规则，否则测试失败并打印未覆盖清单。P1 全量落地时删掉 `*`
//!   兜底规则即自动收紧为"全覆盖或显式 pending"。
//!
//! 解析器（无 yaml crate，纯行扫描）：
//! - OpenAI `openapi.yaml`：2 空格缩进的 `^  /...:` 行为 path，其下 4 空格的
//!   `^    get:/post:/...` 为 method。spec 路径不带 `/v1` 前缀（server URL 里
//!   才带），这里统一补 `/v1`，使 COVERED 与 contract_test 路径一致。
//! - Anthropic `api.md`：`<code title="METHOD path">` 属性 → 去 `?beta=true`
//!   后缀去重（稳定版与 beta 版同端点只计一次）。
//!
//! P0 覆盖九端点对应 Task 9 contract cases 1-11（部分 case 共享端点：非流式/
//! 流式同 path、双协议 GET /v1/models 同 path 不同 provider）。

use std::collections::HashSet;
use std::fs;

const SPECS_DIR: &str = "sebas-gateway/tests/specs";
const OPENAI_SPEC: &str = "openai-openapi.yaml";
const ANTHROPIC_SPEC: &str = "anthropic-api.md";

/// OpenAI yaml 支持的 HTTP method（4 空格缩进 `^    <method>:`）。
const OPENAI_METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

// ===== COVERED：P0 九端点（Task 9 contract cases 1-11 对应）=====

/// Anthropic P0 已覆盖端点（cases 1-5）。
///
/// 注意路径参数名与 spec 一致：Anthropic 用 `{model_id}`。
const COVERED_ANTHROPIC: &[(&str, &str)] = &[
    ("POST", "/v1/messages"),
    ("POST", "/v1/messages/count_tokens"),
    ("GET", "/v1/models"),
    ("GET", "/v1/models/{model_id}"),
];

/// OpenAI P0 已覆盖端点（cases 6-11）。
///
/// OpenAI spec 路径参数用 `{model}`（与 Anthropic 的 `{model_id}` 不同）。
/// 解析时统一补 `/v1` 前缀，故 COVERED 也带 `/v1`。
const COVERED_OPENAI: &[(&str, &str)] = &[
    ("POST", "/v1/chat/completions"),
    ("POST", "/v1/responses"),
    ("POST", "/v1/embeddings"),
    ("GET", "/v1/models"),
    ("GET", "/v1/models/{model}"),
];

// ===== PENDING：显式未覆盖规则（glob, 原因, beads id）=====
//
// P0 期只有 `*` 兜底：除 COVERED 九端点外全部待 P1 全量覆盖。
// P1 落地时删除 `*` 规则，未覆盖端点将必须逐条显式 pending，否则 CI 红。
// P2（难传输层，bead sebas-lva.13）预留给 multipart/uploads/SSE 边界等
// 难以字节级透传的端点——P1 收紧后再按需添加具体 glob 规则。

/// PENDING 规则：`(glob_pattern, 原因, beads_id)`。
///
/// `glob` 用 `*` 作通配（匹配任意字符串），其余字符字面匹配。
/// 一条端点命中任一规则即视为"显式 pending"，不算未覆盖。
const PENDING: &[(&str, &str, &str)] = &[("*", "P1 全量 contract tests", "sebas-lva.12")];

// ===== 解析器 =====

/// 行扫描 OpenAI openapi.yaml → `(METHOD, path)` 清单（path 已补 `/v1` 前缀）。
///
/// 规则：`^  /<path>:`（恰好 2 空格缩进）为 path 行；紧随其后直到下一个 path
/// 行之间，`^    <method>:`（恰好 4 空格缩进）为该 path 下的 method。
///
/// **paths: 作用域限定**：只在顶级 `paths:` 块内收集 endpoint。遇到 0 缩进
/// 顶级 key（如 `webhooks:`、`components:`、`x-oaiMeta:`）即离开 paths 块，
/// `current_path` 置 None，防止后续 4 空格 `    post:` 被误挂到上个 path 上
/// （vendored OpenAI spec 的 `webhooks:` 块有 16 个 `    post:`，不定作用域
/// 会产出 16 个幻影重复端点：304 vs 真实 288）。
fn parse_openai(yaml: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current_path: Option<String> = None;

    for line in yaml.lines() {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        // 0 缩进顶级 key：离开 paths: 块（webhooks:/components:/x-oaiMeta: 等）。
        // 防止上个 path 残留，误吞后续 4 空格 method 行。
        if indent == 0 && trimmed.ends_with(':') && !trimmed.starts_with('#') {
            current_path = None;
            continue;
        }
        // path 行：恰好 2 空格缩进 + 以 / 开头 + 以 : 结尾
        if indent == 2 && trimmed.starts_with('/') && trimmed.ends_with(':') {
            // 去掉末尾 ':'
            let path = &trimmed[..trimmed.len() - 1];
            current_path = Some(format!("/v1{path}"));
            continue;
        }
        // method 行：恰好 4 空格缩进 + 已知 method + ':'，且仍在某 path 下
        if indent == 4
            && let Some(path) = &current_path
        {
            for m in OPENAI_METHODS {
                let needle = format!("{m}:");
                if trimmed == needle {
                    out.push((m.to_ascii_uppercase(), path.clone()));
                    break;
                }
            }
        }
    }
    out
}

/// 行扫描 Anthropic api.md → `(METHOD, path)` 清单（去 `?beta=true` 后缀去重）。
///
/// 规则：每行里 `title="<method> <path>"` 属性，path 可能带 `?beta=true` 后缀。
fn parse_anthropic(md: &str) -> Vec<(String, String)> {
    let mut set: HashSet<(String, String)> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for line in md.lines() {
        // 在一行里可能有多段 title="..."，逐段抠
        let mut rest = line;
        while let Some(start) = rest.find("title=\"") {
            rest = &rest[start + "title=\"".len()..];
            let end = match rest.find('"') {
                Some(e) => e,
                None => break,
            };
            let attr = &rest[..end];
            rest = &rest[end + 1..];
            // attr 形如 "post /v1/messages" 或 "get /v1/models?beta=true"
            let attr = attr.trim();
            let (method, path) = match attr.split_once(' ') {
                Some((m, p)) => (m, p),
                None => continue,
            };
            let method = method.to_ascii_uppercase();
            // 去 ?beta=true（及其它 query 后缀）——稳定版与 beta 版同端点只计一次
            let path = path.split('?').next().unwrap_or(path).to_string();
            let key = (method.clone(), path.clone());
            if set.insert(key) {
                out.push((method, path));
            }
        }
    }
    out
}

// ===== glob 匹配 =====

/// 简易 glob：`*` 匹配任意字符串（含空），其余字符字面匹配。
/// 支持多 `*`。P0 仅 `*` 一条规则；P1 可加更具体 glob。
fn glob_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        match (p.split_first(), t.split_first()) {
            (Some((b'*', p_rest)), _) => {
                // * 匹配空 / 任意长度；尝试所有切分点
                if p_rest.is_empty() {
                    return true; // 末尾 * 吞掉剩余
                }
                // * 跨过 t 的若干字符直到 p_rest 能匹配
                for i in 0..=t.len() {
                    if rec(p_rest, &t[i..]) {
                        return true;
                    }
                }
                false
            }
            (Some((pc, p_rest)), Some((tc, t_rest))) if pc == tc => rec(p_rest, t_rest),
            (Some(_), Some(_)) => false,
            (None, None) => true,
            (Some(_), None) => false,
            (None, Some(_)) => false,
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

/// 端点 key：`METHOD path`（用于匹配 PENDING glob）。
fn endpoint_key(method: &str, path: &str) -> String {
    format!("{method} {path}")
}

// ===== 读取 vendored spec =====

fn read_spec(name: &str) -> String {
    let candidates = [
        format!("{SPECS_DIR}/{name}"),
        format!("sebas-gateway/tests/specs/{name}"),
        format!("tests/specs/{name}"),
    ];
    for p in &candidates {
        if let Ok(s) = fs::read_to_string(p) {
            return s;
        }
    }
    panic!(
        "vendored spec {name} not found under gateway/tests/specs/ (cwd={:?}); \
         run scripts/refresh_api_specs.sh",
        std::env::current_dir().unwrap_or_default()
    );
}

// ===== 解析器自测（fixture 小样本 → 期望清单）=====

#[test]
fn parser_selftest_openai_fixture() {
    let fixture = "\
openapi: 3.1.0
paths:
  /chat/completions:
    get:
      operationId: listChatCompletions
    post:
      operationId: createChatCompletion
  /models:
    get:
      operationId: listModels
  /models/{model}:
    get:
      operationId: retrieveModel
  /responses:
    post:
      operationId: createResponse
components:
  schemas:
    Foo:
      type: object
";
    let got = parse_openai(fixture);
    let want: Vec<(String, String)> = vec![
        ("GET".into(), "/v1/chat/completions".into()),
        ("POST".into(), "/v1/chat/completions".into()),
        ("GET".into(), "/v1/models".into()),
        ("GET".into(), "/v1/models/{model}".into()),
        ("POST".into(), "/v1/responses".into()),
    ];
    assert_eq!(
        got, want,
        "OpenAI parser must extract (METHOD, /v1+path) in document order"
    );
}

/// 回归：`paths:` 之外的 4 空格 method 行不得挂数到上个 path。
///
/// vendored OpenAI spec 的 `webhooks:` 块有 16 个 `    post:`，不定作用域会
/// 全部挂数到最后一个 path（`/v1/responses/input_tokens?beta=true`），产出
/// 16 个幻影重复端点（304 vs 真实 288）。此 fixture 复现该结构。
#[test]
fn parser_selftest_openai_scopes_to_paths_block() {
    let fixture = "\
openapi: 3.1.0
paths:
  /responses:
    post:
      operationId: createResponse
  /responses/input_tokens?beta=true:
    post:
      operationId: createResponseInputTokens
webhooks:
  batch_cancelled:
    post:
      description: Sent when a batch has been cancelled.
  batch_completed:
    post:
      description: Sent when a batch has completed processing.
components:
  schemas:
    WebhookBatchCancelled:
      type: object
x-oaiMeta:
  someMeta: true
";
    let got = parse_openai(fixture);
    let want: Vec<(String, String)> = vec![
        ("POST".into(), "/v1/responses".into()),
        ("POST".into(), "/v1/responses/input_tokens?beta=true".into()),
    ];
    assert_eq!(
        got, want,
        "parse_openai must not attach 4-space method lines outside `paths:` to the last path; \
         the 16 `    post:` lines under `webhooks:` must yield zero phantom endpoints"
    );
}

#[test]
fn parser_selftest_anthropic_fixture() {
    let fixture = "\
# Messages
- <code title=\"post /v1/messages\">client.messages.create</code>
- <code title=\"post /v1/messages/count_tokens\">client.messages.countTokens</code>
# Models
- <code title=\"get /v1/models/{model_id}\">client.models.retrieve</code>
- <code title=\"get /v1/models\">client.models.list</code>
# beta dup (should dedup)
- <code title=\"get /v1/models/{model_id}?beta=true\">client.beta.models.retrieve</code>
- <code title=\"get /v1/models?beta=true\">client.beta.models.list</code>
";
    let got = parse_anthropic(fixture);
    let want: Vec<(String, String)> = vec![
        ("POST".into(), "/v1/messages".into()),
        ("POST".into(), "/v1/messages/count_tokens".into()),
        ("GET".into(), "/v1/models/{model_id}".into()),
        ("GET".into(), "/v1/models".into()),
    ];
    assert_eq!(
        got, want,
        "Anthropic parser must dedup beta variants and preserve first-seen order"
    );
}

#[test]
fn glob_match_star_matches_all() {
    assert!(glob_match("*", "POST /v1/messages"));
    assert!(glob_match("*", ""));
    assert!(glob_match("POST *", "POST /v1/messages"));
    assert!(!glob_match("GET *", "POST /v1/messages"));
    assert!(glob_match("*/v1/models*", "GET /v1/models"));
    assert!(glob_match("*/v1/models*", "GET /v1/models/gpt-4"));
}

// ===== 断言一：COVERED 每个端点必须真实存在于 spec =====

#[test]
fn covered_endpoints_exist_in_specs() {
    let oai_spec = read_spec(OPENAI_SPEC);
    let anth_spec = read_spec(ANTHROPIC_SPEC);
    let oai_set: HashSet<(String, String)> = parse_openai(&oai_spec).into_iter().collect();
    let anth_set: HashSet<(String, String)> = parse_anthropic(&anth_spec).into_iter().collect();

    let mut missing: Vec<String> = Vec::new();
    for (m, p) in COVERED_OPENAI {
        if !oai_set.contains(&(m.to_string(), p.to_string())) {
            missing.push(format!("openai  {m} {p}"));
        }
    }
    for (m, p) in COVERED_ANTHROPIC {
        if !anth_set.contains(&(m.to_string(), p.to_string())) {
            missing.push(format!("anthrop {m} {p}"));
        }
    }
    assert!(
        missing.is_empty(),
        "COVERED 列表含 spec 中不存在的端点（反向防自欺失败）：\n{}\n\
         说明：COVERED 拼写错误或 spec 已变更，需核对后修正 COVERED 或刷新 vendored spec。",
        missing.join("\n")
    );
}

// ===== 断言二：每个 spec 端点要么在 COVERED 要么命中 PENDING =====

#[test]
fn every_spec_endpoint_is_covered_or_pending() {
    let oai_spec = read_spec(OPENAI_SPEC);
    let anth_spec = read_spec(ANTHROPIC_SPEC);
    let oai_all = parse_openai(&oai_spec);
    let anth_all = parse_anthropic(&anth_spec);

    let oai_covered: HashSet<(String, String)> = COVERED_OPENAI
        .iter()
        .map(|(m, p)| (m.to_string(), p.to_string()))
        .collect();
    let anth_covered: HashSet<(String, String)> = COVERED_ANTHROPIC
        .iter()
        .map(|(m, p)| (m.to_string(), p.to_string()))
        .collect();

    let mut uncovered: Vec<String> = Vec::new();

    for (m, p) in &oai_all {
        if oai_covered.contains(&(m.to_string(), p.to_string())) {
            continue;
        }
        let key = endpoint_key(m, p);
        let pending = PENDING.iter().any(|(glob, _, _)| glob_match(glob, &key));
        if !pending {
            uncovered.push(format!("openai  {key}"));
        }
    }
    for (m, p) in &anth_all {
        if anth_covered.contains(&(m.to_string(), p.to_string())) {
            continue;
        }
        let key = endpoint_key(m, p);
        let pending = PENDING.iter().any(|(glob, _, _)| glob_match(glob, &key));
        if !pending {
            uncovered.push(format!("anthrop {key}"));
        }
    }

    eprintln!(
        "spec-diff: openai {} endpoints ({} paths), anthropic {} endpoints; \
         COVERED openai={} anthropic={}; PENDING rules={}",
        oai_all.len(),
        oai_all
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<HashSet<_>>()
            .len(),
        anth_all.len(),
        COVERED_OPENAI.len(),
        COVERED_ANTHROPIC.len(),
        PENDING.len(),
    );

    if !uncovered.is_empty() {
        panic!(
            "spec-diff 发现 {} 个未覆盖且未 pending 的端点（P1 落地需删 PENDING `*` 兜底后逐条 pending）：\n{}\n\
             说明：新增端点要么写进 COVERED 并补 contract test，要么在 PENDING 显式登记。",
            uncovered.len(),
            uncovered.join("\n")
        );
    }
}
