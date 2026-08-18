//! 契约测试（Task 9）：P0 端点字节级透传断言。
//!
//! 表驱动 14 例，每例三断言：
//! ① 下游响应 status/headers/body 与上游 fixture 字节相等；
//! ② mock 记录的入站请求 method/path/body 与发出一致（rename 例外：body 仅断言 model 改写）；
//! ③ 上游收到注入的 key（Anthropic `x-api-key` / OpenAI `authorization: Bearer`）
//!    且下游 `sk-gw-*` key 不出现在任何 header。
//!
//! fixture usage 数字（须与 support 模块一致）：
//! - Anthropic messages：input=10 output=25 cache_read=5 cache_creation=2
//! - OpenAI chat：prompt=12 completion=34
//! - OpenAI responses：input=8 output=20

mod support;

use std::time::Duration;

use gateway::proto::WireProtocol;
use support::*;

// ===== 配置 =====

/// 双协议面标准 config：anthropic + openai 两 provider 指向各自 mock；
/// 两把下游 token（`sk-gw-contract` / `sk-gw-openai`）；三条路由规则
/// （claude-* / gpt-* / text-*）。
fn base_config(anth_url: &str, oai_url: &str) -> String {
    format!(
        r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
default_provider = "anthropic"

auth_token = ["sk-gw-contract", "sk-gw-openai"]

[gateway.routes]
"claude-*" = ["anthropic"]
"gpt-*" = ["openai"]
"text-*" = ["openai"]

[provider.anthropic]
base_url_anthropic = "{anth_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[provider.openai]
base_url_openai = "{oai_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI"
"#
    )
}

/// rename config：在 base 上给 anthropic provider 加 model_map（claude-sonnet → claude-sonnet-4-20250514）。
fn rename_config(anth_url: &str, oai_url: &str) -> String {
    format!(
        r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
default_provider = "anthropic"

auth_token = "sk-gw-contract"

[gateway.routes]
"claude-*" = ["anthropic"]

[provider.anthropic]
base_url_anthropic = "{anth_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[provider.anthropic.model_map]
claude-sonnet = "claude-sonnet-4-20250514"

[provider.openai]
base_url_openai = "{oai_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI"
"#
    )
}

/// openai 默认 config：无 model 的 GET 类请求落到 openai mock
/// （无 per-key default_provider 后，用全局 default_provider 表达）。
fn openai_default_config(anth_url: &str, oai_url: &str) -> String {
    format!(
        r#"
[gateway]
listen = "127.0.0.1:0"
usage_file = "__USAGE__"
default_provider = "openai"

auth_token = "sk-gw-openai"

[gateway.routes]
"claude-*" = ["anthropic"]

[provider.anthropic]
base_url_anthropic = "{anth_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY"

[provider.openai]
base_url_openai = "{oai_url}"
api_key_env = "SEBAS_GATEWAY_TEST_UPSTREAM_KEY_OAI"
"#
    )
}

// ===== 测试环境 =====

struct TestEnv {
    gw: TestGateway,
    anth: MockUpstream,
    oai: MockUpstream,
}

impl TestEnv {
    fn url(&self) -> String {
        format!("http://{}", self.gw.addr)
    }

    fn usage_path(&self) -> std::path::PathBuf {
        self.gw.dir.path().join("usage.jsonl")
    }
}

async fn setup() -> TestEnv {
    let anth = start_mock_upstream(WireProtocol::Anthropic).await;
    let oai = start_mock_upstream(WireProtocol::OpenAi).await;
    let cfg = base_config(&anth.url, &oai.url);
    let gw = start_gateway(&cfg).await;
    TestEnv { gw, anth, oai }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

const ANTH_KEY: &str = "test-anthropic-key";
const OAI_KEY: &str = "test-openai-key";

// ===== 断言辅助 =====

/// ① 下游响应字节级等于 fixture：status=200 + x-mock-trace + content-type + body 字节相等。
async fn assert_response_fixture(
    resp: reqwest::Response,
    fixture: &str,
    trace: &str,
    content_type: &str,
) {
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "status must be 200");
    assert_eq!(
        resp.headers().get("x-mock-trace").unwrap(),
        trace,
        "x-mock-trace header must pass through"
    );
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        content_type,
        "content-type must match fixture"
    );
    let bytes = resp.bytes().await.expect("read response body");
    assert_eq!(
        &bytes[..],
        fixture.as_bytes(),
        "response body must be byte-equal to upstream fixture"
    );
}

/// ② mock 记录的入站请求 method/path/body 与发出一致（字节级）。
fn assert_recorded_request(req: &RecordedRequest, method: &str, path: &str, body: &str) {
    assert_eq!(req.method, method, "recorded method mismatch");
    assert_eq!(req.path, path, "recorded path+query mismatch");
    assert_eq!(
        req.body, body,
        "recorded body must be byte-equal to sent body"
    );
}

/// ③ 上游收到注入的 key + 下游 key 不泄漏。Anthropic 用 x-api-key，OpenAI 用 Authorization: Bearer。
fn assert_key_injection(req: &RecordedRequest, proto: WireProtocol, downstream_key: &str) {
    match proto {
        WireProtocol::Anthropic => {
            assert_eq!(
                recorded_header_get(&req.headers, "x-api-key").unwrap(),
                ANTH_KEY,
                "upstream must receive injected x-api-key"
            );
            assert!(
                recorded_header_get(&req.headers, "authorization").is_none(),
                "downstream Authorization must be stripped for Anthropic"
            );
        }
        WireProtocol::OpenAi => {
            assert_eq!(
                recorded_header_get(&req.headers, "authorization").unwrap(),
                format!("Bearer {OAI_KEY}"),
                "upstream must receive injected Bearer"
            );
            assert!(
                recorded_header_get(&req.headers, "x-api-key").is_none(),
                "x-api-key must be stripped (no upstream injection for OpenAI)"
            );
        }
    }
    for v in req.headers.values() {
        assert!(
            !v.contains(downstream_key),
            "downstream key must not leak to upstream: {v}"
        );
    }
}

/// 取 mock 记录的唯一请求（断言恰好 1 条）。
async fn sole_request(mock: &MockUpstream) -> RecordedRequest {
    let reqs = mock.requests.lock().await.clone();
    assert_eq!(reqs.len(), 1, "expected exactly 1 recorded request");
    reqs.into_iter().next().unwrap()
}

// ===== 1. POST /v1/messages 非流式字节透传（Anthropic mock）=====

#[tokio::test]
async fn case_1_anthropic_messages_non_stream_byte_passthrough() {
    let env = setup().await;
    let client = client();
    let body = r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = client
        .post(format!("{}/v1/messages", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/messages");

    // ① 响应字节级透传
    assert_response_fixture(
        resp,
        ANTHROPIC_MESSAGES_JSON,
        "anthropic-messages-json",
        "application/json",
    )
    .await;

    // ② mock 记录入站请求
    let rec = sole_request(&env.anth).await;
    assert_recorded_request(&rec, "POST", "/v1/messages", body);

    // ③ key 注入 + 不泄漏
    assert_key_injection(&rec, WireProtocol::Anthropic, "sk-gw-contract");
}

// ===== 2. POST /v1/messages 流式 SSE 字节透传 + usage.jsonl 断言 =====

#[tokio::test]
async fn case_2_anthropic_messages_sse_byte_passthrough_and_usage() {
    let env = setup().await;
    let client = client();
    let body =
        r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
    let resp = client
        .post(format!("{}/v1/messages", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/messages stream");

    // ① SSE 响应字节级透传
    assert_response_fixture(
        resp,
        ANTHROPIC_MESSAGES_SSE,
        "anthropic-messages-sse",
        "text/event-stream",
    )
    .await;

    // ② mock 记录入站请求（stream:true 在 body 中）
    let rec = sole_request(&env.anth).await;
    assert_recorded_request(&rec, "POST", "/v1/messages", body);

    // ③ key 注入 + 不泄漏
    assert_key_injection(&rec, WireProtocol::Anthropic, "sk-gw-contract");

    // usage.jsonl 断言：input=10 output=25 cache_read=5 cache_creation=2
    let records = poll_usage_jsonl(&env.usage_path(), 1).await;
    let u = &records[0];
    assert_eq!(u["protocol"], "anthropic");
    assert_eq!(u["status"], 200);
    assert_eq!(u["input_tokens"], 10);
    assert_eq!(u["output_tokens"], 25);
    assert_eq!(u["cache_read_tokens"], 5);
    assert_eq!(u["cache_creation_tokens"], 2);
}

// ===== 3. POST /v1/messages/count_tokens =====

#[tokio::test]
async fn case_3_anthropic_count_tokens_byte_passthrough() {
    let env = setup().await;
    let client = client();
    let body = r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = client
        .post(format!("{}/v1/messages/count_tokens", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/messages/count_tokens");

    assert_response_fixture(
        resp,
        ANTHROPIC_COUNT_TOKENS,
        "anthropic-count-tokens",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.anth).await;
    assert_recorded_request(&rec, "POST", "/v1/messages/count_tokens", body);
    assert_key_injection(&rec, WireProtocol::Anthropic, "sk-gw-contract");
}

// ===== 4. GET /v1/models（带 anthropic-version → Anthropic mock）=====

#[tokio::test]
async fn case_4_get_models_anthropic_routes_to_anthropic_mock() {
    let env = setup().await;
    let client = client();
    let resp = client
        // Claude Code /model 的请求形态：带 ?limit 查询串。
        .get(format!("{}/v1/models?limit=1000", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .expect("GET /v1/models");

    assert_response_fixture(
        resp,
        ANTHROPIC_MODELS_LIST,
        "anthropic-models-list",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.anth).await;
    assert_recorded_request(&rec, "GET", "/v1/models?limit=1000", "");
    assert_key_injection(&rec, WireProtocol::Anthropic, "sk-gw-contract");
    // 路由断言：openai mock 未收到任何请求
    let oai_reqs = env.oai.requests.lock().await;
    assert!(
        oai_reqs.is_empty(),
        "openai mock must not receive a request"
    );
}

// ===== 5. GET /v1/models/{id}（Anthropic）=====

#[tokio::test]
async fn case_5_get_model_by_id_anthropic_byte_passthrough() {
    let env = setup().await;
    let client = client();
    let resp = client
        .get(format!("{}/v1/models/claude-sonnet-4-20250514", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .expect("GET /v1/models/{id}");

    assert_response_fixture(
        resp,
        ANTHROPIC_MODEL_GET,
        "anthropic-model-get",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.anth).await;
    assert_recorded_request(&rec, "GET", "/v1/models/claude-sonnet-4-20250514", "");
    assert_key_injection(&rec, WireProtocol::Anthropic, "sk-gw-contract");
}

// ===== 6. POST /v1/chat/completions 非流式（OpenAI mock）=====

#[tokio::test]
async fn case_6_openai_chat_non_stream_byte_passthrough() {
    let env = setup().await;
    let client = client();
    let body = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = client
        .post(format!("{}/v1/chat/completions", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/chat/completions");

    assert_response_fixture(
        resp,
        OPENAI_CHAT_JSON,
        "openai-chat-json",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.oai).await;
    assert_recorded_request(&rec, "POST", "/v1/chat/completions", body);
    assert_key_injection(&rec, WireProtocol::OpenAi, "sk-gw-contract");
}

// ===== 7. POST /v1/chat/completions 流式 + usage 断言 =====

#[tokio::test]
async fn case_7_openai_chat_sse_byte_passthrough_and_usage() {
    let env = setup().await;
    let client = client();
    let body = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}],"stream":true}"#;
    let resp = client
        .post(format!("{}/v1/chat/completions", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/chat/completions stream");

    assert_response_fixture(
        resp,
        OPENAI_CHAT_SSE,
        "openai-chat-sse",
        "text/event-stream",
    )
    .await;

    let rec = sole_request(&env.oai).await;
    assert_recorded_request(&rec, "POST", "/v1/chat/completions", body);
    assert_key_injection(&rec, WireProtocol::OpenAi, "sk-gw-contract");

    // usage 断言：prompt=12 completion=34 → input=12 output=34
    let records = poll_usage_jsonl(&env.usage_path(), 1).await;
    let u = &records[0];
    assert_eq!(u["protocol"], "openai");
    assert_eq!(u["status"], 200);
    assert_eq!(u["input_tokens"], 12);
    assert_eq!(u["output_tokens"], 34);
    assert!(
        u["cache_read_tokens"].is_null(),
        "OpenAI must not set cache_read_tokens"
    );
    assert!(u["cache_creation_tokens"].is_null());
}

// ===== 8. POST /v1/responses（responses usage shape）=====

#[tokio::test]
async fn case_8_openai_responses_byte_passthrough_and_usage() {
    let env = setup().await;
    let client = client();
    let body = r#"{"model":"gpt-4","input":"hi"}"#;
    let resp = client
        .post(format!("{}/v1/responses", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/responses");

    assert_response_fixture(
        resp,
        OPENAI_RESPONSES_JSON,
        "openai-responses-json",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.oai).await;
    assert_recorded_request(&rec, "POST", "/v1/responses", body);
    assert_key_injection(&rec, WireProtocol::OpenAi, "sk-gw-contract");

    // usage 断言：responses shape input=8 output=20
    let records = poll_usage_jsonl(&env.usage_path(), 1).await;
    let u = &records[0];
    assert_eq!(u["protocol"], "openai");
    assert_eq!(u["input_tokens"], 8);
    assert_eq!(u["output_tokens"], 20);
}

// ===== 9. POST /v1/embeddings =====

#[tokio::test]
async fn case_9_openai_embeddings_byte_passthrough() {
    let env = setup().await;
    let client = client();
    let body = r#"{"model":"text-embedding-3-small","input":"hi"}"#;
    let resp = client
        .post(format!("{}/v1/embeddings", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/embeddings");

    assert_response_fixture(
        resp,
        OPENAI_EMBEDDINGS,
        "openai-embeddings",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.oai).await;
    assert_recorded_request(&rec, "POST", "/v1/embeddings", body);
    assert_key_injection(&rec, WireProtocol::OpenAi, "sk-gw-contract");
}

// ===== 10. GET /v1/models（无 anthropic-version → OpenAI mock）=====

#[tokio::test]
async fn case_10_get_models_no_header_routes_to_openai_mock() {
    let anth = start_mock_upstream(WireProtocol::Anthropic).await;
    let oai = start_mock_upstream(WireProtocol::OpenAi).await;
    let cfg = openai_default_config(&anth.url, &oai.url);
    let gw = start_gateway(&cfg).await;
    let env = TestEnv { gw, anth, oai };
    let client = client();
    let resp = client
        .get(format!("{}/v1/models", env.url()))
        .header("authorization", "Bearer sk-gw-openai")
        .send()
        .await
        .expect("GET /v1/models no header");

    assert_response_fixture(
        resp,
        OPENAI_MODELS_LIST,
        "openai-models-list",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.oai).await;
    assert_recorded_request(&rec, "GET", "/v1/models", "");
    assert_key_injection(&rec, WireProtocol::OpenAi, "sk-gw-openai");
    // 路由断言：anthropic mock 未收到任何请求
    let anth_reqs = env.anth.requests.lock().await;
    assert!(
        anth_reqs.is_empty(),
        "anthropic mock must not receive a request"
    );
}

// ===== 11. GET /v1/models/{id}（OpenAI）=====

#[tokio::test]
async fn case_11_get_model_by_id_openai_byte_passthrough() {
    let env = setup().await;
    let client = client();
    let resp = client
        .get(format!("{}/v1/models/gpt-4", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .send()
        .await
        .expect("GET /v1/models/gpt-4");

    assert_response_fixture(
        resp,
        OPENAI_MODEL_GET,
        "openai-model-get",
        "application/json",
    )
    .await;

    let rec = sole_request(&env.oai).await;
    assert_recorded_request(&rec, "GET", "/v1/models/gpt-4", "");
    assert_key_injection(&rec, WireProtocol::OpenAi, "sk-gw-contract");
}

// ===== 12. 显式前缀：/anthropic/v1/messages 与 /openai/v1/chat/completions 剥离前缀转发 =====

#[tokio::test]
async fn case_12_explicit_prefix_strips_and_forwards() {
    let env = setup().await;
    let client = client();

    // /anthropic/v1/messages → Anthropic mock 收到 /v1/messages
    let body_a = r#"{"model":"claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#;
    let resp_a = client
        .post(format!("{}/anthropic/v1/messages", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("content-type", "application/json")
        .body(body_a)
        .send()
        .await
        .expect("POST /anthropic/v1/messages");
    assert_response_fixture(
        resp_a,
        ANTHROPIC_MESSAGES_JSON,
        "anthropic-messages-json",
        "application/json",
    )
    .await;
    let rec_a = sole_request(&env.anth).await;
    // 前缀剥离：mock 收到的 path 是 /v1/messages（非 /anthropic/v1/messages）
    assert_recorded_request(&rec_a, "POST", "/v1/messages", body_a);
    assert_key_injection(&rec_a, WireProtocol::Anthropic, "sk-gw-contract");

    // /openai/v1/chat/completions → OpenAI mock 收到 /v1/chat/completions
    let body_o = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#;
    let resp_o = client
        .post(format!("{}/openai/v1/chat/completions", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("content-type", "application/json")
        .body(body_o)
        .send()
        .await
        .expect("POST /openai/v1/chat/completions");
    assert_response_fixture(
        resp_o,
        OPENAI_CHAT_JSON,
        "openai-chat-json",
        "application/json",
    )
    .await;
    let rec_o = sole_request(&env.oai).await;
    // 前缀剥离：mock 收到的 path 是 /v1/chat/completions
    assert_recorded_request(&rec_o, "POST", "/v1/chat/completions", body_o);
    assert_key_injection(&rec_o, WireProtocol::OpenAi, "sk-gw-contract");
}

// ===== 13. model rename：provider model_map 命中，上游收到改写后的 model =====

#[tokio::test]
async fn case_13_model_rename_rewrites_upstream_model_field() {
    let anth = start_mock_upstream(WireProtocol::Anthropic).await;
    let oai = start_mock_upstream(WireProtocol::OpenAi).await;
    let cfg = rename_config(&anth.url, &oai.url);
    let gw = start_gateway(&cfg).await;
    let env = TestEnv { gw, anth, oai };
    let client = client();

    // 客户端发 model=claude-sonnet；model_map 改写为 claude-sonnet-4-20250514
    let body = r#"{"model":"claude-sonnet","messages":[{"role":"user","content":"hi"}]}"#;
    let resp = client
        .post(format!("{}/v1/messages", env.url()))
        .header("authorization", "Bearer sk-gw-contract")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST /v1/messages rename");

    // ① 响应字节级透传（fixture 不受 rename 影响）
    assert_response_fixture(
        resp,
        ANTHROPIC_MESSAGES_JSON,
        "anthropic-messages-json",
        "application/json",
    )
    .await;

    // ② rename 例外：body 仅断言 model 字段改写 + 其它字段保留
    let rec = sole_request(&env.anth).await;
    assert_eq!(rec.method, "POST");
    assert_eq!(rec.path, "/v1/messages");
    let upstream_body: serde_json::Value =
        serde_json::from_str(&rec.body).expect("upstream body is JSON");
    assert_eq!(
        upstream_body["model"], "claude-sonnet-4-20250514",
        "model must be renamed via model_map"
    );
    assert_eq!(
        upstream_body["messages"][0]["content"], "hi",
        "other fields preserved"
    );

    // ③ key 注入 + 不泄漏
    assert_key_injection(&rec, WireProtocol::Anthropic, "sk-gw-contract");
}
