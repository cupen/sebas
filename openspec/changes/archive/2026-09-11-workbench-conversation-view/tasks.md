## 0. 前置条件

- [x] 0.1 确认 `workbench-turn-queue` 已合并（提交只在开轮进入 transcript 的不变量必须成立）；验证：`openspec list` 中该 change 已归档，或对应提交已在 `main` 上。

## 1. wire：条目序列与工具标签（design D1/D2）

- [x] 1.1 `sebas-webui/src/api.rs` 的 detail 组装改为输出有序 `entries`（`position`/`kind`/`element_type`/`content`/`created_at_unix`），不再过滤 `kind == "prompt"`；验证：webui api 测试断言两侧条目同时出现且顺序正确。
- [x] 1.2 退役 `user_prompt` 与 `body` 两个字段，并同步 `models.rs` 的响应结构；验证：api 测试断言响应中不再有这两个字段（旧字段出现即失败）。
- [x] 1.3 `sebas-dispatch/src/engine/mod.rs` 的 `ToolStart`/`ToolEnd` 两条 push 点写 `element_type = "tool"`（内容仍为可读 markdown）；验证：`sebas-dispatch` 单测断言条目类型为 `tool`，且 `turn-content` 检索结果里工具与正文可区分。
- [x] 1.4 `GET /api/summary` 的聚焦会话 payload 与本序列对齐（同一形状或明确指向 detail 取全量）；验证：api 测试断言 summary 与 detail 对同一会话给出一致的条目视图。

## 2. 对话视图（design D3/D4/D5）

- [x] 2.1 把 `views/transcript-view.ts` 演进为对话视图：按 `kind == prompt` 切回合，一个 agent 回合渲染一个气泡；验证：组件单测「N 条 chunk → 1 个气泡」。
- [x] 2.2 实现回合内分块规则：连续正文条目按 position 拼接成文本段、thinking 收进折叠块、`tool` 条目收进「用了 N 个工具」可展开组，并按位置落在文本段之间；验证：组件单测覆盖「正文→工具→正文」得到三段式结构。
- [x] 2.3 operator 回合渲染（「你」气泡，复用既有 `is-user` 样式）并按 position 与 agent 回合交替；验证：组件单测断言两侧交替顺序与提交文本。
- [x] 2.4 seam 改为按回合计数、锚定边界下方首个回合的 position，永不落在回合内部；验证：组件单测「一个含多 chunk 的回合只计 1 条未读，且边界不切开该回合」。
- [x] 2.5 空态 / 加载 / 错误 / a11y 门禁随视图更新（含新增工具组的可键盘展开）；验证：`pnpm test`（含 `a11y.test.ts`）通过。

## 3. 面合一：dashboard 为唯一对话面（design D5/D6）

- [x] 3.1 rail 点击改为「switch + 就地聚焦」：`POST /api/sessions/{key}/switch` 后停在 `/`，dashboard 渲染该会话；验证：`project-rail.test.ts` + 浏览器 e2e「点会话后仍在工作台」。
- [x] 3.2 rail 当前标记改由焦点指针驱动（不再比较 `location.pathname`）；验证：`project-rail.test.ts` 断言焦点变化时标记跟随。
- [x] 3.3 Close、归档入口与 `sebas-review-cards` 搬进 dashboard 的会话头（从 `session-detail.ts` 迁移）；验证：dashboard 组件单测 + 浏览器 e2e「在 dashboard 关闭/归档/review-card 可达」。
- [x] 3.4 `session-detail.ts` 退休；`/sessions/{key}` 深链渲染 dashboard 并聚焦该会话；验证：`router.test.ts` + 浏览器 e2e「书签深链仍打开该会话」。
- [x] 3.5 退役 `session-detail.ts` 的残留引用与测例（CSS、导入、路由分支），保持 typecheck 干净；验证：`pnpm run tsc` 与 `pnpm test` 通过，`grep -r "sebas-session-detail"` 无生产引用。

## 4. 模型选择器接 Settings 目录（design D7/D8）

- [x] 4.1 新增 BFF `GET /router/api/defaults`（透传 router `/admin/defaults`，未设置返回双 null）；验证：webui api 测试覆盖正常与 router 不可达时的降级响应。
- [x] 4.2 SPA 侧新增 adapter `toModelCatalog(providers, defaults) → {provider, model}[]`，只做规整、不解释结构；验证：单测覆盖空目录、provider 无 `models`、默认值不在目录内三种输入。
- [x] 4.3 创建模式的 composer 改为两级选择（先 provider 后 model），默认项取 adapter 的默认；会话存在时切换为会话 `available_models`，为空则不给下拉且不报错；验证：`workbench-composer.test.ts` 覆盖三种模式与「目录不可用时显式禁用而不伪造选项」。
- [x] 4.4 目录读取失败（router 不在跑）落到显式「目录不可用」态，不显示空列表；验证：组件单测 + 浏览器 e2e。

## 5. 门禁与验收

- [x] 5.1 `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test --workspace` 全绿；验证：命令输出无 error。（注：clippy 与 cargo test 全绿——1347 passed；`cargo fmt --check` 在仓库存量上本就不绿（与本 change 无关的 examples/、sebas-acp/ 等历史债务，经 stash 对照验证为基线既有），本 change 触碰的全部 Rust 文件已单独 rustfmt 校验通过。）
- [x] 5.2 前端门禁：`pnpm run tsc` 与 `pnpm test` 全绿；验证：命令输出。
- [x] 5.3 进程级 e2e：`invoke testsuite-e2e` 全绿（含条目 `kind`/`element_type` 透出用例）；验证：任务输出摘要。
- [x] 5.4 浏览器 e2e：`invoke testsuite-webui-server` 覆盖「对话两侧交替」「工具组展开」「就地聚焦」「模型两级选择」四条旅程；验证：任务输出摘要。（注：四条旅程在 `tests/testsuite-webui/tests/conversation.spec.ts` + 既有用例更新后全绿；整套 41/42，唯一失败 `errors.spec` 的 refuse 旅程是**基线既有缺陷**（turn-queue 未提交改动的非终端错误收尾从不生效，拒绝回合卡 WORKING），经二进制二分定位与本 change 无关，见汇报。）
- [x] 5.5 文档与术语一致性：`openspec validate workbench-conversation-view` 通过；若引入「回合/turn」术语则先更新 `openspec/glossary.md`；验证：命令输出 + 词条存在。
