## 1. 模型条目形状与兼容

- [x] 1.1 把 provider 的模型列表由字符串列表改为条目列表（`id` + 能力标记），并加兼容反序列化：裸字符串读作仅 `text` 的条目；验证：单测覆盖「旧字符串列表可读且无错」「写回为条目」
- [x] 1.2 静态 preset 表的模型列表同步改为条目并携带能力；验证：单测断言从 preset 读到带 `vision` 的条目
- [x] 1.3 保证顺序语义不变：默认模型取首条条目的 id，env 映射按条目 id 解析；验证：既有 models / env 映射单测通过，并新增「首条即默认」断言
- [x] 1.4 固化能力词表与落盘规则：`text` 隐含不入库，`vision` / `audio` / `video` 显式，未知标记按明确行为处理（拒绝或忽略，二选一并测到）；验证：单测覆盖三者

## 2. 设置弹窗交互修复

- [x] 2.1 给设置弹窗内全部 `@wa-hide` 绑定加事件源判断（`e.target === e.currentTarget`），覆盖 provider 编辑器、设为默认、删除、服务确认、全部重启、重置；验证：vitest 从子 `<wa-select>` 派发 `wa-hide` 后编辑器保持打开
- [x] 2.2 保留既有关闭路径（关闭按钮 / Escape / 点遮罩）为回归；验证：`settings-modal.test.ts` 既有三条关闭用例仍通过

## 3. Provider 表单重做

- [x] 3.1 预制表单收敛为「选 provider + API key + 模型条目（可增删、可勾选能力）」，实例名默认取预设名；验证：vitest 断言提交载荷含条目与能力标记、不含 `base_url_*` 与 `api_key_env`
- [x] 3.2 定制表单加实例名 / base_url / 协议，其余 URL 槽与模型改名映射收进默认折叠的 Advanced；验证：vitest 断言默认折叠、展开后可编辑其余槽位、默认提交不带这些字段
- [x] 3.3 表单重做时保留 `add-fetch-models` 已落地的抓取入口（机制与 UI 归该 change，本处不重建、不重定义）：抓取入口在预制与定制表单中仍然可达，无可用 base url 仍不渲染；验证：vitest 断言重做后的表单仍能触发抓取，且本 change 未新增抓取语义
- [x] 3.4 从 Models 分区删除 Router 网关卡（listen / debug / auth）；验证：vitest 断言 Models 渲染不再发起 `/api/router` 请求且不出现网关卡

## 4. Services 分区承载 router 状态

- [x] 4.1 确认 Services 呈现 router 的 desired / actual / uptime，无 watchdog 形态如实降级；验证：vitest 断言 router 行渲染、`adapter_ok:false` 时呈现横幅而非用空列表冒充

## 5. 测试与门禁

- [x] 5.1 更新 Playwright：`tests/testsuite-webui/tests/models.spec.ts` 的「provider 只读 + 零 probe 流量」断言改为「可抓取、可编辑模型条目」；`settings.spec.ts` 的 provider 表单用例随新载荷更新；验证：`invoke testsuite-webui-server` 后 `pnpm playwright test` 全绿
- [x] 5.2 前端单测与类型门禁；验证：`pnpm -C sebas-webui/frontend test` 与 typecheck 均通过
- [x] 5.3 Rust 门禁；验证：`cargo test --all-targets` 与 `cargo clippy --all-targets` 全绿
- [x] 5.4 链式复核：确认 `make-core-own-provider-data` 与 `add-fetch-models` 已归档后再应用本 change，并重跑 `openspec validate --strict`；验证：校验通过且端到端可走通「建 provider → 抓取 → 挑模型 → 设默认 → 起会话」（`invoke testsuite-webui-sandbox`）
