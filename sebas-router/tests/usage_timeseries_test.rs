//! `/admin/usage/timeseries` 集成测试（add-usage-statistics tasks 1.2/1.3）。
//!
//! 覆盖：admin 鉴权同面（secret 在场无 Bearer 401、错 Bearer 401 不回显、
//! 无 secret loopback 放行）、双模型日聚合（spec 场景「two models aggregate
//! into separate day buckets」）、未观测 token 只计请求数、参数口径（非法
//! 400 / 越界 clamp / 缺省 14 天与 UTC）、小时粒度 24 桶且不含昨天。桶切分
//! 的时区数学由 `usage_query.rs` 纯函数单测钉住，这里钉端点装配与参数管道。

mod support;

use std::time::Duration;

use sebas_db::writer::StateWriter;
use sebas_router::usage::{USAGE_TABLES, UsageRow};
use serde_json::Value;
use support::start_router;

const CFG_TMPL: &str = r#"
[router]
listen = "127.0.0.1:0"
usage_db = "__USAGE__"

[provider.anthropic]
api_key_env = "SEBAS_ROUTER_TEST_UPSTREAM_KEY"
"#;

/// 401 用例要控制 `SEBAS_CONTROL_SECRET`（进程 env，admin_auth 逐请求读取），
/// 与其它可能改它的测试串行。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

/// dev-deps 的 reqwest 未开 `json` feature：统一 text + serde_json 解析。
async fn get_json(client: &reqwest::Client, url: &str) -> Value {
    let text = client
        .get(url)
        .send()
        .await
        .expect("get timeseries")
        .text()
        .await
        .expect("body text");
    serde_json::from_str(&text).expect("json body")
}

/// 直接向 router 的 usage.db 插入行（绕过 sink 通道，聚合测试只要库里
/// 有数据）。表结构经 `StateWriter::start` 的既有 schema 同步补齐；插入走
/// 单写 actor 的同一命令队列（与生产写入同一路径）。
async fn insert_rows(db: &std::path::Path, rows: &[UsageRow]) {
    let writer = StateWriter::start(db.to_path_buf(), USAGE_TABLES).expect("open usage db");
    for row in rows {
        writer.handle().save(row).await.expect("insert usage row");
    }
}

fn sample_row(ts: String, model: Option<&str>, input: Option<i64>, output: Option<i64>) -> UsageRow {
    UsageRow {
        id: None,
        key: String::new(),
        protocol: "anthropic".into(),
        model: model.map(str::to_string),
        provider: "anthropic".into(),
        upstream_model: None,
        status: 200,
        latency_ms: 1,
        ttft_ms: None,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: None,
        cache_creation_tokens: None,
        error: None,
        ts,
    }
}

/// 今天 12:00 UTC（RFC3339）：窗口内的确定性时间戳——不取 `now - 2h`，避免
/// 午夜前后跑测试时「几小时前」落到昨天导致窗口/日期桶翻车（聚合窗口没有
/// 上界，未来时间戳同样落今天桶，测试意图不受影响）。
fn today_noon() -> String {
    chrono::Utc::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .expect("12:00 exists")
        .and_utc()
        .to_rfc3339()
}

fn bucket_models<'a>(body: &'a Value, label: &str) -> &'a Vec<Value> {
    body["buckets"]
        .as_array()
        .expect("buckets array")
        .iter()
        .find(|b| b["bucket"] == label)
        .unwrap_or_else(|| panic!("bucket {label} missing"))
        ["models"]
        .as_array()
        .expect("models array")
}

#[tokio::test(flavor = "multi_thread")]
async fn timeseries_endpoint_requires_the_admin_bearer_when_secret_is_set() {
    // admin_auth 逐请求读 `SEBAS_CONTROL_SECRET`：启动后置 env 即可让本端点
    // 与 `/admin/stats` 同落「Bearer 必填」档（401 谓词与其它 /admin/* 完全
    // 一致——spec 场景「endpoint requires admin auth」）。
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let router = start_router(CFG_TMPL).await;
    unsafe {
        std::env::set_var("SEBAS_CONTROL_SECRET", "ctrl-test-secret");
    }
    let client = client();
    let url = format!("http://{}/admin/usage/timeseries", router.addr);

    // 无 Bearer → 401。
    let resp = client.get(&url).send().await.expect("get timeseries");
    assert_eq!(resp.status(), 401, "secret 在场则 Bearer 必填");
    // 错 Bearer → 401（不回显）。
    let resp = client
        .get(&url)
        .bearer_auth("wrong-secret")
        .send()
        .await
        .expect("get timeseries");
    assert_eq!(resp.status(), 401);
    let text = resp.text().await.expect("body");
    assert!(!text.contains("ctrl-test-secret"), "401 不得回显 secret");
    // 对 Bearer → 200。
    let resp = client
        .get(&url)
        .bearer_auth("ctrl-test-secret")
        .send()
        .await
        .expect("get timeseries");
    assert_eq!(resp.status(), 200);

    // 拆掉 secret 恢复 loopback 姿势：同端点无需 Bearer 即放行（standalone
    // 部署姿势零改动）。
    unsafe {
        std::env::remove_var("SEBAS_CONTROL_SECRET");
    }
    let resp = client.get(&url).send().await.expect("get timeseries");
    assert_eq!(resp.status(), 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_models_aggregate_into_separate_day_buckets_and_zero_fill() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let router = start_router(CFG_TMPL).await;
    let db = router.dir.path().join("usage.db");
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let base = today_noon();
    insert_rows(
        &db,
        &[
            sample_row(base.clone(), Some("claude-sonnet"), Some(10), Some(50)),
            sample_row(base, Some("gpt-4o-mini"), Some(5), Some(8)),
        ],
    )
    .await;

    let client = client();
    let body = get_json(
        &client,
        &format!(
            "http://{}/admin/usage/timeseries?granularity=day&days=3",
            router.addr
        ),
    )
    .await;

    assert_eq!(body["granularity"], "day");
    assert_eq!(body["days"], 3);
    let buckets = body["buckets"].as_array().expect("buckets");
    assert_eq!(buckets.len(), 3, "窗口逐日零填充");
    let models = bucket_models(&body, &today);
    assert_eq!(models.len(), 2, "同日双模型分列（spec 场景）");
    let sonnet = models
        .iter()
        .find(|m| m["model"] == "claude-sonnet")
        .expect("claude-sonnet present");
    assert_eq!(sonnet["input_tokens"], 10);
    assert_eq!(sonnet["output_tokens"], 50);
    let gpt = models
        .iter()
        .find(|m| m["model"] == "gpt-4o-mini")
        .expect("gpt-4o-mini present");
    assert_eq!(gpt["input_tokens"], 5);
    assert_eq!(gpt["output_tokens"], 8);
    // 其余日期零填充：models 为空数组。
    for b in buckets {
        if b["bucket"] != today {
            assert!(b["models"].as_array().unwrap().is_empty());
        }
    }
    // 汇总 = 各模型之和。
    assert_eq!(body["totals"]["requests"], 2);
    assert_eq!(body["totals"]["input_tokens"], 15);
}

#[tokio::test(flavor = "multi_thread")]
async fn unobserved_tokens_add_requests_only() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let router = start_router(CFG_TMPL).await;
    let db = router.dir.path().join("usage.db");
    let ts = today_noon();
    // 第二行 = 上游错误行：token 未观测（NULL）、status 502。
    let mut failed = sample_row(ts.clone(), Some("m"), None, None);
    failed.status = 502;
    failed.error = Some("boom".into());
    insert_rows(&db, &[sample_row(ts, Some("m"), Some(10), Some(5)), failed]).await;

    let client = client();
    let body = get_json(
        &client,
        &format!("http://{}/admin/usage/timeseries", router.addr),
    )
    .await;
    let models = bucket_models(
        &body,
        &chrono::Utc::now().format("%Y-%m-%d").to_string(),
    );
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["requests"], 2, "失败行计入请求数");
    assert_eq!(models[0]["input_tokens"], 10, "None 不计入 token 和");
    assert_eq!(models[0]["output_tokens"], 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn param_validation_is_enforced_at_the_endpoint() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let router = start_router(CFG_TMPL).await;
    let client = client();
    let base = format!("http://{}/admin/usage/timeseries", router.addr);

    // 非法 granularity → 400。
    let resp = client
        .get(format!("{base}?granularity=week"))
        .send()
        .await
        .expect("get");
    assert_eq!(resp.status(), 400);
    let text = resp.text().await.expect("body");
    assert!(text.contains("granularity"), "400 点名非法参数: {text}");

    // 非数字 days / tz_offset → 400。
    for q in ["?days=abc", "?tz_offset=xyz"] {
        let resp = client.get(format!("{base}{q}")).send().await.expect("get");
        assert_eq!(resp.status(), 400, "query {q}");
    }

    // 越界 clamp：days=999 → 30；tz_offset=-99999 → -840；缺省 14/0。
    let body = get_json(&client, &format!("{base}?days=999&tz_offset=-99999")).await;
    assert_eq!(body["days"], 30);
    assert_eq!(body["tz_offset"], -840);
    let default_body = get_json(&client, &base).await;
    assert_eq!(default_body["days"], 14, "缺省窗口 14 天");
    assert_eq!(default_body["tz_offset"], 0, "缺省偏移 UTC");
    assert_eq!(
        default_body["buckets"].as_array().unwrap().len(),
        14,
        "缺省窗口零填充 14 个日期桶"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hour_granularity_returns_24_buckets_of_today() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let router = start_router(CFG_TMPL).await;
    let db = router.dir.path().join("usage.db");
    // 今天 12:00 UTC 一条 + 两天前一条：小时面只应有今天那条（spec「hour
    // query ignores yesterday」）。
    let recent = today_noon();
    let old = (chrono::Utc::now() - chrono::Duration::days(2)).to_rfc3339();
    insert_rows(
        &db,
        &[
            sample_row(recent, Some("m"), Some(7), None),
            sample_row(old, Some("m"), Some(9), None),
        ],
    )
    .await;

    let client = client();
    let body = get_json(
        &client,
        &format!(
            "http://{}/admin/usage/timeseries?granularity=hour",
            router.addr
        ),
    )
    .await;
    let buckets = body["buckets"].as_array().expect("buckets");
    assert_eq!(buckets.len(), 24, "0–23 全 24 桶");
    assert_eq!(body["granularity"], "hour");
    let total_requests: u64 = buckets
        .iter()
        .map(|b| {
            b["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["requests"].as_u64().unwrap_or(0))
                .sum::<u64>()
        })
        .sum();
    assert_eq!(total_requests, 1, "昨天的小时明细不进今天的小时桶");
}
