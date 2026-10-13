## 1. 桥文件生成与 provider 注册表映射

- [x] 1.1 新增 `src/pi_provider_bridge.rs`：`pub struct ProviderBridge`、`pub fn render_models_json(cfg: &Config, providers: &ProvidersSnapshot) -> serde_json::Value`、`pub fn write_bridge(sebas_home: &Path, model_json: &Value) -> Result<PathBuf>`。Verify：`cargo test -p sebas pi_provider_bridge::tests::render_anthropic_minimal` 通过；空 provider 快照时返回空 JSON 对象。

- [x] 1.2 `src/run.rs` 装配点调用 `write_bridge`：读 settings.db providers 快照，按 D3 规则生成 models.json（`base_url_anthropic` → anthropic 块、`base_url_openai_chat`/`openai_responses` → openai 块；密钥 `${SEBAS_PI_KEY_n}` n 从 0 起；同协议族默认赢、其他跳过并 `tracing::warn!`）。Verify：临时 home 跑一个 fake-claude 形态的 spec，inspect `<SEBAS_HOME>/pi-agent/models.json` 的 `anthropic.baseUrl` 等于 `base_url_anthropic` 的值；`default_selection.provider` 切换时文件覆盖写。

- [x] 1.3 `sebas-acp/src/pi/driver.rs::build_command` 注入 `PI_CODING_AGENT_DIR` env。Verify：`build_command` 单元测试覆盖 `DriverConfig.extra_env` 含 `PI_CODING_AGENT_DIR`；fake-pi 桩能跑出 `EnvGet` 日志含该变量。

- [x] 1.4 删除 `src/spawn_env.rs::compute_pi_spawn_env` 及 `src/session_boot.rs::spawn_overrides` 的 pi 分支调用。`resolve_pi_spawn_env` 留 `#[deprecated(since = "make-pi-ride-sebas-provider", note = "use ProviderBridge + models.json instead")]` 标记，调用点改返回空。Verify：`grep -R compute_pi_spawn_env src/` 仅匹配 deprecation 注释；`cargo build -p sebas` 通过。

- [x] 1.5 回滚 shim：`pub fn legacy_pi_env_shim_enabled() -> bool` 读 `SEBAS_LEGACY_PI_ENV` 切换。Verify：`SEBAS_LEGACY_PI_ENV=1 cargo test -p sebas` 通过（e2e 备用）；默认 false。

## 2. Core 会话通道 StateSnapshot 扩展

- [x] 2.1 `sebas-ipc/src/protocol.rs`：`StateSnapshot` 响应帧 payload 增 `execution_bodies: Option<HashMap<String, ExecutionBodyStatus>>`（默认 None，旧客户端忽略）。`ExecutionBodyStatus = { ok: bool, cause: Option<String> }` 同模块新增。Verify：`cargo test -p sebas-ipc` 通过；fixture 含新字段 round-trip。

- [x] 2.2 core 端 `src/dispatch.rs` 的 `StateSnapshot { domain: "agents" }` 响应里读 `DualSessionBackend::execution_bodies()`（`sebas-webui` crate）并序列化。Verify：单元测试覆盖有/无 execution_bodies 两态；core 实测 `StateSnapshot agents` payload 含 `execution_bodies.native`。

- [x] 2.3 `src/core_channel/client.rs` 的 `CoreChannelBackend`：加 `execution_bodies` 字段缓存 + 内部 `async fn refresh_execution_bodies()`（经 `StateSnapshot` 拉一次），覆写 `SessionBackend::execution_bodies()` 返回缓存；`Changed { scope: "agents" }` 推送触发刷新。Verify：`core_channel::tests::webui_execution_bodies_resolves_from_state_snapshot`（mock core 状态，验证 `execution_bodies()` 返回该状态）通过。

## 3. 前端 + pi driver 文案

- [x] 3.1 `sebas-webui/frontend/src/views/new-session-dialog.ts::agentUnavailableLabel` 删除 `a.id === 'native'` 硬编码分支，改读 `a.cause`；保留通用非 native 行的 fallback 文案。Verify：`new-session-dialog.test.ts` 加 `native unavailable label uses catalog cause not hardcoded literal`：mock 两条 catalog（"未配 provider" / "pi 二进制不可达"）断言 label 与 cause 一致、且**不**含硬编码"未安装 pi"。

- [x] 3.2 `sebas-acp/src/pi/driver.rs` 在合适时机（如 handshake 后或独立 `probe_auth` 任务）调 `pi auth check --provider anthropic --model <default>`，把 stdout 三态（ready/not_ready/invalid）映射为 `ExecutionBodyStatus { ok, cause }`。Verify：fake-pi 桩模拟 stdout 三态，单元测试覆盖；现有 `agent_kinds_test::native_body_*` 测试不破。

- [x] 3.3 `sebas-webui/src/agent_kinds.rs::pi_auth_cause`：去掉"stdout 任何非 ready/not_ready/invalid 行 → `pi auth check: <other>`"兜底（带 `--provider/--model` 后 pi 必返三态之一），保留 cause 净化（去 env 变量名）。Verify：现有 `pi_auth_cause_advisory_*` 测试不破；新增 `pi_auth_cause_drops_env_var_names_from_message` 通过。

## 4. 回归与 e2e

- [x] 4.1 迁移 `tests/testsuite_e2e_test.rs::native_slot_runs_pi_with_provider_env_and_model_switch` → 改名为 `native_slot_runs_pi_via_sebas_bridge`：经 settings.db 注入 fake provider 行（`base_url_anthropic` 指 fake 上游），spawn 设 `SEBAS_PI_KEY_0` + `PI_CODING_AGENT_DIR`，期望 fake 上游收到 POST + Bearer key。Verify：测试通过；`fake-pi-journal.log` 含目标 `get_available_models` 与 prompt 流。

- [x] 4.2 新增 `tests/testsuite_e2e_test.rs::detached_topology_native_reachability_transmitted`：sandbox 用 `sebas run` 起 core + webui 两进程（参考 `TESTSUITE_MODE=detached`），① pi 二进制在场 + provider 配置时 webui 拉 `GET /api/agents` 见 native `reachable: true`；② provider 清除后再拉见 `reachable: false` cause `"未配 provider"`。Verify：两条断言分别通过。

- [x] 4.3 `tests/pi_driver_integration_test.rs` 加 `spawn_overrides_no_longer_sets_anthropic_env`：spawn pi 子进程，dump 子进程 `/proc/<pid>/environ`，断言不含 `ANTHROPIC_API_KEY` / `ANTHROPIC_BASE_URL` / `ANTHROPIC_MODEL` 由 sebas 注入（同名 env 出现次数为 0）。Verify：测试通过；现有 fake-pi 桩 e2e 不破。

- [x] 4.4 `sebas-acp/src/pi/driver.rs` 单元测试更新 fake-pi 桩的 `auth check` 应答（带 `--provider/--model` 后 stdin 形状变化）。Verify：fake-pi 脚本更新；`pi_driver_integration_test` 的 auth-cause 相关测试通过。

## 5. 协议与跨包一致性

- [x] 5.1 `sebas-ipc/src/protocol.rs` 的 `execution_bodies` schema 同步登记到 `ipc_protocol_home` fixture（`tests/fixtures/ipc_core_channel_wire.json` 加一条带 execution_bodies 的 StateSnapshot 响应行）。Verify：`cargo test -p sebas-ipc ipc_protocol_home_test` 通过，fixture 对比无 diff（按 forward-compat 规则）。

- [x] 5.2 `sebas-webui/src/api.rs::agent_kinds` 在 disconnected/core 不可达分支 native 行仍为 `reachable: false` cause 含"核心不可达"——不变（向后兼容）。Verify：`agent_kinds_test.rs` 现有 disconnected case 测试不破。

- [x] 5.3 `AGENTS.md` 沙箱小节补一段：sebas 私有 `<SEBAS_HOME>/pi-agent/` 与 `pi-agent/` 内 `models.json` / `SEBAS_PI_KEY_n` 的关系；sandbox 必须 `SEBAS_HOME` 钉到 `<SB>` 并确保 `<SB>/pi-agent/` 存在。Verify：`grep "pi-agent\|pi_agent" AGENTS.md` ≥ 1 处。

## 6. 收口

- [x] 6.1 `cargo build --workspace` / `cargo test --workspace` / `cargo clippy --workspace -- -D warnings` 通过。Verify：退出码 0。

- [x] 6.2 `openspec validate make-pi-ride-sebas-provider --strict` 通过。Verify：退出码 0，无 schema 警告。

- [x] 6.3 本 change 不在实现阶段归档——`openspec list` 显示 `make-pi-ride-sebas-provider status=in-progress`，等用户显式 `openspec archive`。Verify：`openspec list --json` 输出中本 change 在 in-progress。
