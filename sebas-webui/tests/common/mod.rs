//! Shared integration-test support for the `sebas-webui` endpoint binaries.
//!
//! `migrate-project-registry` deleted the `projects.json` file backend: the
//! core state store is now the only authority for projects, and
//! `SEBAS_PROJECTS_PATH` is retired. Test binaries whose app is built on
//! `InProcessBackend` therefore need a real `StateStoreEngine` installed into
//! the process-global store (`sebas_dispatch::state_store`), because that
//! backend reads projects straight from `engine()`.
//!
//! This module supplies an in-memory engine with real projects CRUD (plus
//! settings and persisted-state storage) and two process-global helpers:
//!
//! - [`init`] installs the engine once per test process (idempotent);
//! - [`reset_projects`] clears the project registry so a test observes exactly
//!   the projects it registers itself.
//!
//! Files under `tests/common/` are modules, not test binaries: include it with
//! `mod common;` from each test file that needs it.

#![allow(dead_code)]

use sebas_dispatch::state_store::{PersistedState, StateStoreEngine};
use sebas_models::project::ProjectRow;
use std::sync::{Arc, Mutex, Once, OnceLock};

/// The process-global in-memory state: persisted runtime state, settings, and
/// the project registry.
#[derive(Default)]
struct MemoryInner {
    state: Mutex<PersistedState>,
    settings: Mutex<Option<serde_json::Value>>,
    projects: Mutex<Vec<ProjectRow>>,
}

struct MemoryEngine {
    inner: Arc<MemoryInner>,
}

/// The single shared inner store for this test process.
fn shared() -> Arc<MemoryInner> {
    static INNER: OnceLock<Arc<MemoryInner>> = OnceLock::new();
    INNER
        .get_or_init(|| Arc::new(MemoryInner::default()))
        .clone()
}

/// Install the in-memory engine as the process-global core state store.
///
/// Idempotent: the global `OnceLock` can only be initialised once per process,
/// so every test may call this freely.
pub fn init() {
    static INIT: Once = Once::new();
    let inner = shared();
    INIT.call_once(|| {
        sebas_dispatch::state_store::init_engine(Box::new(MemoryEngine { inner }));
    });
}

/// Clear the in-memory project registry.
///
/// Per-test isolation for the shared engine: callers must hold their
/// per-binary serialization lock around `reset_projects()` + the assertions,
/// because the registry is process-global.
pub fn reset_projects() {
    shared().projects.lock().unwrap().clear();
}

#[async_trait::async_trait]
impl StateStoreEngine for MemoryEngine {
    async fn load_persisted_state(&self) -> PersistedState {
        self.inner.state.lock().unwrap().clone()
    }

    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()> {
        *self.inner.state.lock().unwrap() = state;
        Ok(())
    }

    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String> {
        Ok(self.inner.settings.lock().unwrap().clone())
    }

    async fn save_settings(&self, cfg: serde_json::Value) -> Result<(), String> {
        *self.inner.settings.lock().unwrap() = Some(cfg);
        Ok(())
    }

    /// Registry order mirrors the production query: `sort_order, added_at`.
    async fn load_projects(&self) -> Result<Vec<ProjectRow>, String> {
        let mut projects = self.inner.projects.lock().unwrap().clone();
        projects.sort_by_key(|p| (p.sort_order, p.added_at));
        Ok(projects)
    }

    async fn save_projects(&self, projects: Vec<ProjectRow>) -> Result<(), String> {
        *self.inner.projects.lock().unwrap() = projects;
        Ok(())
    }

    /// Add is idempotent by path (the production registry keys on `path`).
    /// The stable id is derived here exactly as the wire derives it, so
    /// `set_project_default_agent` can address the row by its wire id.
    async fn add_project(
        &self,
        node_id: &str,
        path: &str,
        name: &str,
        added_at: i64,
    ) -> Result<(), String> {
        let mut projects = self.inner.projects.lock().unwrap();
        if projects.iter().any(|p| p.path == path) {
            return Ok(());
        }
        projects.push(ProjectRow {
            id: Some(sebas_webui::projects::project_id_for_on(node_id, path)),
            path: path.to_string(),
            name: name.to_string(),
            branch_at: 0,
            added_at,
            sort_order: 0,
            node_id: node_id.to_string(),
            default_agent: None,
            branch: None,
        });
        Ok(())
    }

    async fn remove_project(&self, path: &str) -> Result<bool, String> {
        let mut projects = self.inner.projects.lock().unwrap();
        let before = projects.len();
        projects.retain(|p| p.path != path);
        Ok(projects.len() != before)
    }

    /// Address the row by stored id, falling back to the `(node, path)`
    /// derived id (the wire id is always the derived one).
    async fn set_project_default_agent(&self, id: &str, agent: &str) -> Result<(), String> {
        let mut projects = self.inner.projects.lock().unwrap();
        for p in projects.iter_mut() {
            let matches = p.id.as_deref() == Some(id)
                || sebas_webui::projects::project_id_for_on(&p.node_id, &p.path) == id;
            if matches {
                p.default_agent = Some(agent.to_string());
            }
        }
        Ok(())
    }
}
