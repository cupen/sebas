# Tasks — remove-superpowers-docs

## 1. 同步:ADR 文档

- [x] 1.1 新建 `docs/design-history.md`,搭好 ADR 格式(日期/背景/决策/后果/原文路径);写 D1 清单中的第 1–3 条(弃 ACP 直连、卡片流选型、gateway 双协议面与 per-key 简化),蒸馏自对应 superpowers 文档,每条 10–20 行。验证:三条决策的原文路径能在 `git log --follow` 中找到对应文件
- [x] 1.2 续写第 4–6 条(provider state v2 统一、provider 评审决策记录摘要、watchdog 控制平面分期),其中第 5 条覆盖「routes 后续由 webui 编辑」承诺。验证:决策记录中 12/15 落地状态与未竟项与原文 §5 一致

## 2. 同步:遗留事项入 bead

- [x] 2.1 逐项核对 `docs/review/2026-08-17-code-design-audit.md` 的 5 条建议现状(P1 webui loopback 拦截、P2 service_status 硬编码、P2 run_watchdog 双 ControlService、P3 过渡注释、P3 降级日志);已修的在 design-history.md 附记,未修的开 bead。验证:`bd list` 可见对应 issue,P1 在列且描述含现状证据(`webui_cmd.rs:66`)
- [x] 2.2 为「gateway TOML routes 后续由 webui 编辑、TOML routes 配置移除」开 bead,描述链接 design-history.md 对应条目。验证:`bd show <id>` 内容完整

## 3. 清理引用:显式路径(12 处)

- [x] 3.1 README.md 两处(`docs/superpowers/specs/2026-07-26-sebas-design.md`、`2026-08-06-gateway-design.md`)改指 `openspec/specs/` 对应 capability(cli-service/feishu-cards、gateway-core 等)与 config.toml.example。验证:`grep -n "superpowers" README.md` 零命中
- [x] 3.2 config/config.toml.example 两处注释改指 `openspec/specs/feishu-cards`、`openspec/specs/gateway-core`;src/cli.rs、src/gateway_cmd.rs、gateway/src/lib.rs 三处模块注释同步改指。验证:`grep -rn "docs/superpowers" src/ gateway/ config/` 零命中
- [x] 3.3 acp-claude 三处(manager.rs/session.rs/driver.rs)改指 `acp-driver` spec;router/src/router/maps.rs 的 per-turn quote 引用改指 `feishu-reactions` spec;docs/perm-flow/sequence.md 改指 `permission-flow` spec。验证:`grep -rn "docs/superpowers" acp-claude/ router/ docs/` 零命中(archive 目录除外)

## 4. 清理引用:带日期 `spec 2026-08-17 §N`(64 处,15 文件 + how-to.md)

- [x] 4.1 gateway 系(gateway/src/config.rs、debug.rs、models.rs、key_resolver.rs、proto.rs):决策已落地的删引用标签保留结论,仍描述现行行为的改指 `gateway-*` spec。验证:抽查 3 处上下文,替换指向正确;家族内零残留
- [x] 4.2 router 系(state_store.rs、crud.rs、provider_state.rs、provider_card.rs、tests/provider_test.rs)与 src 系(spawn_env.rs、gateway_cmd.rs、provider.rs、session_boot.rs):同规则处理。验证:抽查 3 处;`grep -rn "spec 2026-08-17" src/ router/ gateway/` 零命中
- [x] 4.3 acp-claude/src/agent_driver.rs(§2.1/§2.2/§2.5 注释)与 .claude/rules/how-to.md(§2.8 一处)清理;后者仅删引用标签、其余内容不动。验证:`grep -rn "spec 2026-" . --include="*.rs" --include="*.md"` 除 archive 外零命中

## 5. 清理引用:裸 `spec §N`(133 处,46 文件,按家族)

- [x] 5.1 卡片流家族(card_state.rs、card_events.rs、provider_card.rs、acp_events.rs、inbound.rs、commands.rs、state.rs、feishu/cards.rs、dispatch.rs、run.rs、tests/{pump_unit,full_e2e,config_env,restart_recovery}_test.rs):指向 `feishu-cards` spec,纯实现细节就地内联或删标签。验证:抽查 5 处上下文与指向一致
- [x] 5.2 gateway 家族(gateway/src/{proxy,auth,routing,sse,usage,error,proto}.rs、tests/{auth,proxy_smoke}_test.rs):按端点/鉴权/用量分别指向 `gateway-core`/`gateway-auth-rate-limit`/`gateway-metrics`。验证:抽查 5 处
- [x] 5.3 provider 与命令/控制面家族(src/{config,main,record,session_boot}.rs、src/watchdog/{control_rpc,executor}.rs、cli.rs、acp-claude/{manager,driver}.rs、tests/resume_session_test.rs、router/tests/*):apply 时先核对 `spec §12`/`§4.1` 等实际出处再映射到 `cli-service`/`watchdog`/`acp-driver`;无对应 spec 的内联事实或删标签。验证:`grep -rn "spec §" src/ router/ gateway/ acp-claude/ feishu/ tests/` 零命中

## 6. 防回归:xtask check-docs

- [x] 6.1 xtask 新增 `check-docs` 子命令:扫描 `*.rs|*.toml|*.md`(排除 `target/`、`openspec/changes/archive/`、`docs/design-history.md`),命中 `docs/superpowers/`、`spec \d{4}-\d{2}-\d{2}`、`spec §\d` 即非零退出并报告文件:行号;附命中/豁免/干净三个单元测试。验证:`cargo run -p xtask -- check-docs` 在清理完成的树上通过,在含样例引用的临时树上失败

## 7. 删除文档

- [x] 7.1 确认 1.x、2.x 全部完成后,删除 `docs/superpowers/`(23 文件)与 `docs/review/2026-08-17-code-design-audit.md`,`docs/review/` 目录随之移除。验证:`ls docs/` 只剩 `perm-flow/` 与 `design-history.md`;`git log --oneline -- docs/superpowers | head -1` 可考
- [x] 7.2 全量验证:`cargo run -p xtask -- check-docs` 零命中;`cargo test` 全绿(注释修改不引入编译错误);README/docs 入口指向正确。验证:三条命令输出符合预期

## 8. 收尾

- [ ] 8.1 按 Conventional Commits 分批提交(同步/清理各批/工具/删除),每批可独立构建;删除 commit 置于最后。验证:`git log` 顺序符合 design D5
