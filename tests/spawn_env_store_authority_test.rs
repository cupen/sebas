//! spawn 权威修正（make-core-own-provider-data 4.1）：`read_overlay_item`
//! 不再优先读 legacy `providers.json`——状态库是权威。文件与库不一致时
//! **以库为准**（回归：旧实现里 stale 文件会盖住库里的 `default_model`）。
//!
//! 独立测试二进制的原因（同 `state_channel_contract_test.rs`）：ENGINE 是
//! 进程级 OnceLock，lib 单测进程不能初始化（会污染依赖「engine 未初始化走
//! 文件回退」的并行测试）；这里显式初始化 fake engine，并把 env 重定向到
//! 一次性目录。`state_store::load()` 走 `block_in_place`，必须用
//! multi_thread 运行时。

use sebas_dispatch::provider_state::{ProviderMode, ProviderRuntimeState};
use sebas_dispatch::state_store::{DefaultSelection, PersistedState};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Once, OnceLock};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// fake engine 的可检视 providers 表。
#[derive(Default)]
struct FakeInner {
    providers: Mutex<BTreeMap<String, serde_json::Map<String, serde_json::Value>>>,
    deleted: Mutex<Vec<String>>,
}

struct FakeEngine {
    inner: Arc<FakeInner>,
}

fn fake_inner() -> Arc<FakeInner> {
    static INNER: OnceLock<Arc<FakeInner>> = OnceLock::new();
    static INIT: Once = Once::new();
    let inner = INNER.get_or_init(|| Arc::new(FakeInner::default()));
    INIT.call_once(|| {
        sebas_dispatch::state_store::init_engine(Box::new(FakeEngine {
            inner: inner.clone(),
        }));
    });
    inner.clone()
}

#[async_trait::async_trait]
impl sebas_dispatch::state_store::StateStoreEngine for FakeEngine {
    async fn load_persisted_state(&self) -> PersistedState {
        PersistedState {
            providers: self.inner.providers.lock().unwrap().clone(),
            deleted: self.inner.deleted.lock().unwrap().clone(),
            ..PersistedState::default()
        }
    }
    async fn save_persisted_state(&self, _state: PersistedState) -> anyhow::Result<()> {
        Ok(())
    }
    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }
    async fn save_settings(&self, _cfg: serde_json::Value) -> Result<(), String> {
        Ok(())
    }
    async fn load_projects(&self) -> Result<Vec<serde_json::Value>, String> {
        Ok(Vec::new())
    }
    async fn save_projects(&self, _projects: Vec<serde_json::Value>) -> Result<(), String> {
        Ok(())
    }
    async fn add_project(&self, _path: &str, _name: &str, _added_at: i64) -> Result<(), String> {
        Ok(())
    }
    async fn remove_project(&self, _path: &str) -> Result<bool, String> {
        Ok(true)
    }
    async fn set_project_default_agent(&self, _id: &str, _agent: &str) -> Result<(), String> {
        Ok(())
    }
}

/// seed 库里的 provider（带 default_model）。
fn seed_store_provider(name: &str, default_model: &str) {
    let inner = fake_inner();
    let mut item = serde_json::Map::new();
    item.insert(
        "base_url_anthropic".into(),
        serde_json::json!("https://store.example/anthropic"),
    );
    item.insert("api_key".into(), serde_json::json!("sk-store"));
    item.insert("default_model".into(), serde_json::json!(default_model));
    inner.providers.lock().unwrap().insert(name.into(), item);
}

/// 写一份与库**不一致**的 legacy providers.json（同名条目、不同 default_model）。
fn write_conflicting_file(dir: &std::path::Path, name: &str, default_model: &str) {
    let doc = serde_json::json!({
        "providers": { name: {
            "base_url_anthropic": "https://file.example/anthropic",
            "api_key": "sk-file",
            "default_model": default_model,
        }},
        "deleted": [],
    });
    let path = dir.join("providers.json");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(&path, doc.to_string()).unwrap();
    // SAFETY: ENV_LOCK 全程持有。
    unsafe {
        std::env::set_var("SEBAS_ROUTER_PROVIDER_OVERLAY", path.to_str().unwrap());
    }
}

/// Direct 模式下 spawn 解析出的 `--model`（default_model 的投递路径）。
fn direct_spawn_model(provider: &str) -> Option<String> {
    let state = ProviderRuntimeState {
        mode: ProviderMode::Direct {
            provider: provider.into(),
        },
        default_selection: Some(DefaultSelection::new(provider)),
    };
    let (resolution, model) = sebas::spawn_env::compute_provider_resolution(&state, None);
    assert!(
        matches!(resolution, sebas_acp::claude::ProviderResolution::Direct { .. }),
        "expected Direct resolution, got {resolution:?}"
    );
    model
}

/// 文件与库不一致（同名 provider、不同 default_model）→ 以库为准。
#[tokio::test(flavor = "multi_thread")]
async fn store_beats_conflicting_legacy_file() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    seed_store_provider("alpha", "store-model");
    write_conflicting_file(dir.path(), "alpha", "file-model");

    // read_overlay_item 经 compute_provider_resolution 投递 default_model：
    // 必须是库里的 store-model，不是文件的 file-model。
    assert_eq!(
        direct_spawn_model("alpha"),
        Some("store-model".into()),
        "文件与库不一致时以库为准（4.1）"
    );
}

/// 库里 tombstone 的 provider：即使 legacy 文件仍有条目，也视同不存在。
#[tokio::test(flavor = "multi_thread")]
async fn store_tombstone_beats_legacy_file_entry() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    {
        let inner = fake_inner();
        inner.deleted.lock().unwrap().push("gone".into());
    }
    write_conflicting_file(dir.path(), "gone", "file-model");

    let state = ProviderRuntimeState {
        mode: ProviderMode::Direct {
            provider: "gone".into(),
        },
        default_selection: Some(DefaultSelection::new("gone")),
    };
    let (resolution, _model) = sebas::spawn_env::compute_provider_resolution(&state, None);
    assert!(
        matches!(resolution, sebas_acp::claude::ProviderResolution::Off),
        "库墓碑必须压过文件条目，got {resolution:?}"
    );
}

/// 库为空（无该条目）而文件有 → 不再从文件救场：视同「找不到」，回退
/// router_cfg（这里没有 → Off + warn）。
#[tokio::test(flavor = "multi_thread")]
async fn file_only_entry_no_longer_shadows_missing_store_row() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    fake_inner(); // engine 已初始化（共享 fake），providers 为空
    write_conflicting_file(dir.path(), "ghost", "file-model");

    let state = ProviderRuntimeState {
        mode: ProviderMode::Direct {
            provider: "ghost".into(),
        },
        default_selection: Some(DefaultSelection::new("ghost")),
    };
    let (resolution, model) = sebas::spawn_env::compute_provider_resolution(&state, None);
    assert!(
        matches!(resolution, sebas_acp::claude::ProviderResolution::Off),
        "store 缺条目时文件不得再充当数据源，got {resolution:?}"
    );
    assert_eq!(model, None);
}
