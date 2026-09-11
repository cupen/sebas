## 1. core 抓取能力

- [x] 1.1 把 `sebas-router/src/admin.rs` 的 `fetch_models` 抽成可复用实现（URL 候选顺序、5s 超时、`data[].id` 解析、错误净化），供 core 调用；验证：抽取后 `cargo test -p sebas-router` 既有 probe 单测仍通过（改为直测抽取函数）
- [x] 1.2 在 core 的 `providers` 域新增抓取 op：以 provider 名解析 base url 与密钥，执行一次只读 GET，返回 id 列表；验证：`tests/state_*` 新增用例断言「成功返回 ids」「无 base url 回 typed rejection」「上游错误回净化 reason」
- [x] 1.3 断言抓取不写任何字段：调用前后 provider 快照逐字节一致；验证：单测比较 before/after 快照
- [x] 1.4 预制派生 provider 可抓取（用代码表 base url）；验证：单测断言 preset 派生 provider 无存储 URL 时抓取仍发起

## 2. 飞书卡片探测改承载

- [x] 2.1 `/provider` 卡片的探测改调 core 抓取 op，保持「custom 写回、preset 派生只读呈现」语义；验证：`sebas-dispatch/tests/provider_test.rs` 与 im 前端用例通过
- [x] 2.2 卡片错误卡只显示净化后的原因；验证：单测断言结果与错误文本不含密钥

## 3. WebUI 抓取入口

- [x] 3.1 provider 表单增加抓取动作与结果列表，对预制与定制都渲染，无可用 base url 时不渲染；验证：vitest 断言两种模式都能触发、无 URL 时不渲染按钮
- [x] 3.2 挑选某个抓取结果才写入模型列表，写入时只带隐含的文字能力、参数标记为本地解析或未知；验证：vitest 断言抓取本身不提交任何写请求，挑选后才提交
- [x] 3.3 失败如实呈现（显示净化原因，不显示空列表冒充「该 provider 没有模型」）；验证：vitest 以 mock 失败断言错误态渲染

## 4. 测试与门禁

- [x] 4.1 更新 Playwright `tests/testsuite-webui/tests/models.spec.ts`：原「零 probe 流量 / provider 只读」断言改为「抓取可用且不改库、挑选后才写」；验证：`invoke testsuite-webui-server` 后 `pnpm playwright test` 全绿
- [x] 4.2 前端单测与类型门禁；验证：`pnpm -C sebas-webui/frontend test` 与 typecheck 通过
- [x] 4.3 Rust 门禁；验证：`cargo test --all-targets` 与 `cargo clippy --all-targets` 全绿
- [x] 4.4 端到端联调：sandbox 内对预制 provider 抓取、挑选一个模型、设为默认、起会话；验证：`invoke testsuite-webui-sandbox` 手工走通，或 `invoke testsuite-e2e --case <provider 相关用例>` 通过
