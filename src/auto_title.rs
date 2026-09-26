//! 会话自动标题的触发编排（add-agent-settings-and-session-titles 6.2）。
//!
//! 触发点有二，都汇到本模块：① `Out::WebSpawn` 携带非空 prompt 的 spawn
//! 成功之后（spawn 带 prompt 与「占位会话首条消息」经 SpawnNew 的到达线）；
//! ② `Out::PlaceholderFirstTurn`——空 prompt 激活的占位会话收到第一条真实
//! 消息（Continue 路由开轮）。`tokio::spawn` 异步生成，**不阻塞回合路径**；
//! provider/model 取 providers 域 `default_selection`，生成与清洗在
//! [`sebas_dispatch::title`]，写回经 `DispatchHandle::web_set_auto_title`
//! （label 仍空才落）。
//!
//! 失败语义（spec「falls back silently」）：未配默认 provider/model、条目缺
//! URL、网络/HTTP/超时失败、清洗后为空——一律静默放弃，首条消息预览顶上；
//! 不重试、不上报操作员。resume 不触发（dormant 会话已有命名来源，且每会话
//! 只在首条消息时尝试一次）。native 内核会话不经此路径（其 spawn 不走
//! `Out::WebSpawn`，命名由 native 侧既有链路承载）。

use sebas_channels::ChannelKey;
use sebas_dispatch::DispatchHandle;

/// 在后台生成并写回标题。调用方（spawn 成功点）只管投递，任何后续失败都
/// 在本任务内静默消化。
pub fn spawn_auto_title(
    router: DispatchHandle,
    key: ChannelKey,
    first_message: String,
) {
    tokio::spawn(async move {
        let Some(title) = generate(&first_message).await else {
            return;
        };
        // 写回：label 仍空才落（操作员在生成窗口内改名 → 标题丢弃）。
        router.web_set_auto_title(key, title).await;
    });
}

/// 解析默认 provider → 目标 → 一次标题调用。任何一环缺失 → `None`。
async fn generate(first_message: &str) -> Option<String> {
    let engine = sebas_dispatch::state_store::engine()?;
    let state = engine.load_persisted_state().await;
    let selection = state.default_selection?;
    let model = selection
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let item = state.providers.get(&selection.provider)?;
    let target = sebas_dispatch::title::resolve_title_target(item, model)?;
    sebas_dispatch::title::generate_title(&sebas_dispatch::title::title_client(), &target, first_message)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 未配默认 provider（store 缺省态）→ 静默无标题（不 panic、不发请求）。
    #[tokio::test]
    async fn no_default_provider_yields_no_title() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        assert!(generate("hello").await.is_none());
    }

    /// 默认模型缺省（只配了 provider 名）→ 静默无标题——标题不猜模型。
    #[tokio::test]
    async fn default_selection_without_model_yields_no_title() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        let engine = sebas_dispatch::state_store::engine().unwrap();
        sebas_dispatch::state_store::settings_mutation(
            engine,
            &serde_json::json!({"op": "set_defaults", "provider": "deepseek"}),
        )
        .await
        .unwrap();
        assert!(generate("hello").await.is_none());
    }

    /// 未知 provider 名（default_selection 指向不存在的条目）→ 静默无标题。
    #[tokio::test]
    async fn unknown_default_provider_yields_no_title() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        let engine = sebas_dispatch::state_store::engine().unwrap();
        sebas_dispatch::state_store::settings_mutation(
            engine,
            &serde_json::json!({"op": "set_defaults", "provider": "ghost", "model": "m"}),
        )
        .await
        .unwrap();
        assert!(generate("hello").await.is_none());
    }

    /// 不可达上游 → None（静默回退路径闭环；HTTP 200 形状解析由 title 模块
    /// 的 mock 上游单测覆盖）。
    #[tokio::test]
    async fn unreachable_upstream_yields_no_title() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        let engine = sebas_dispatch::state_store::engine().unwrap();
        sebas_dispatch::state_store::settings_mutation(
            engine,
            &serde_json::json!({"op": "set_defaults", "provider": "unreachable", "model": "m"}),
        )
        .await
        .unwrap();
        sebas_dispatch::state_store::providers_mutation(
            engine,
            &serde_json::json!({
                "op": "put",
                "name": "unreachable",
                "item": {"base_url_anthropic": "http://127.0.0.1:1", "api_key": "sk-x"},
            }),
        )
        .await
        .unwrap();
        assert!(generate("hello").await.is_none(), "不可达上游静默放弃");
    }
}
