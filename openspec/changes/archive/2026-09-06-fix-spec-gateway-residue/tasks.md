## 1. 应用主 specs 措辞纠正

- [x] 1.1 `openspec/specs/agent-core/spec.md`:LLM channel 需求及场景 gateway→router(3 处),与 delta 逐字一致(early-sync);`grep -in "gateway" openspec/specs/agent-core/spec.md` 确认无残留
- [x] 1.2 `openspec/specs/provider-management/spec.md`:卡片布局按钮 `Gateway`→`Router`、场景标题 gateway mode write / gateway never pins model / gateway env construction 随改、Mode switching 的 pre-rename 状态值条款改为拒绝语义;主 spec 改到与 delta 逐字一致;grep 确认仅剩拒绝场景中的旧值引用
- [x] 1.3 `openspec/specs/webui/spec.md`:3 个场景标题改名(gateway page reflects live state → router data reflects live state、gateway mutations unavailable without secret → router mutations…、gateway mutation is post-only and origin-checked → router mutation…),主 spec 改到与 delta 逐字一致;退役路径 `/gateway` 句原样保留;grep 确认仅剩 retired-path 提及
- [x] 1.4 `openspec/specs/watchdog/spec.md`:Control request surface 服务列表 `(webui, gateway)`→`(webui, router)`、场景标题 gateway managed when enabled → router managed when enabled;主 spec 改到与 delta 逐字一致;grep 确认无残留
- [x] 1.5 `openspec/specs/cli-service/spec.md`:子命令树删除隐藏别名 watchdog/gateway 及理由句(保留 status/services/ctl 现行别名)、env 需求删除 pre-rename honored 条款,改为 SHALL NOT 被接受/读取;主 spec 改到与 delta 逐字一致(early-sync);grep 确认旧 env 名仅出现在 SHALL NOT 条款与拒绝场景中

## 2. 拆除代码兼容层(BREAKING)

- [x] 2.1 删除旧 env 名回退:`src/provider.rs:322`(SEBAS_GATEWAY_PROVIDER_OVERLAY)、`src/agent_backend.rs:167-185`(SEBAS_AGENT_GATEWAY_URL / SEBAS_AGENT_GATEWAY_AUTH)连同告警,及 `src/agent_backend.rs:1000` 错误文案中的旧名;验证:相关单测改为断言旧名不生效,`grep -rn "SEBAS_GATEWAY\|SEBAS_AGENT_GATEWAY" src sebas-*/src` 归零(注释/文案除外)
- [x] 2.2 删除配置节迁移 shim:`src/config.rs:514-522`([gateway]→[router]、[watchdog.gateway]→[watchdog.router])、`src/config.rs:508`([router]→[dispatch])、`sebas-router/src/config.rs:1785-1787`,连同解析前重写与告警;相关测试删除断言旧行为的用例;验证:`cargo test` 通过,旧节名配置按未知节错误路径失败
- [x] 2.3 删除 provider 状态值 serde alias:`sebas-dispatch/src/provider_state.rs:30-31`(`#[serde(alias = "gateway")]`);验证:单测断言 `"kind": "gateway"` 解析报错
- [x] 2.4 删除隐藏 CLI 别名:`src/cli.rs:26`(`alias = "gateway"`)、`src/cli.rs:40`(`alias = "watchdog"`);验证:clap 测试断言 `sebas gateway` / `sebas watchdog` 报未知子命令
- [x] 2.5 文档随迁:`openspec/glossary.md` 去掉"旧名…现为隐藏别名"描述、`tests/acceptance/COVERAGE.md` 的 gateway-* 能力名(router-admin-api 等)与 `SEBAS_AGENT_GATEWAY_URL` 备注、`AGENTS.md` sandbox 配方核对(已用新名,确认即可)

## 3. 验证与归档

- [x] 3.1 全量扫描:`grep -rniE "\bgateway\b" openspec/specs/` 仅剩 retired-path 与 SHALL NOT/拒绝场景条款;`grep -rn "SEBAS_GATEWAY\|SEBAS_AGENT_GATEWAY\|alias = \"gateway\"\|watchdog.gateway" src sebas-*/src tests` 归零
- [x] 3.2 `cargo build && cargo test`(全量)+ `openspec validate fix-spec-gateway-residue --strict` 通过(主 specs 与 delta 逐字一致后即绿)
- [x] 3.3 归档本 change(archive):主 specs 已 early-sync,MODIFIED 应全部识别为 already-in-sync 跳过,确认零场景丢失(HTTP route surface 10 场景、Mutation posture 3、Managed service table 3 等数量不变)
