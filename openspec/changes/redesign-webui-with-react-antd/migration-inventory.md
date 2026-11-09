# 迁移盘点（migration inventory）

- 盘点基线：`feat/react-webui-redesign` @ `40d8ce3`（2026-11-08），工作区有 27 个与本 change 无关的 Rust 改动，未纳入本盘点，也未修改。
- 代码事实来源：`sebas-webui/frontend/src/`、`tests/testsuite-webui/tests/`、`openspec/specs/`、`tests/acceptance/COVERAGE.md`。
- 规模：前端实现 24,055 行（TS/CSS，不含测试）、colocated 测试 23,112 行（40 个 `*.test.ts`），合计 47,167 行；浏览器套件 71 个 `*.spec.ts`。
- 本盘点只回答「有什么、归谁、复用/重写/删除」，不重做 proposal/design 的范围决策。

## 1. 路由与深链

| 路由 id | pattern | 渲染出口 | 关键语义 | 来源 |
|---|---|---|---|---|
| `dashboard` | `/` | `sebas-dashboard` | 唯一对话面；未选项目/未聚焦显示空态 | `src/app-shell.ts:73`、`:968` |
| `sessions` | `/sessions` | `sebas-sessions` | 跨项目会话总览（1080px 文档列） | `src/app-shell.ts:75` |
| `session-deep-link` | `/sessions/:key` | 同一个 `sebas-dashboard`，经 `deepLinkKey` 聚焦 | `:key` 是 **RAW percent-encoded** 段（含 `%00`），不得提前解码；不生成独立详情页 | `src/app-shell.ts:77`、`src/router.ts:1-12` |
| `usage` | `/usage` | `sebas-usage` | 独立可深链用量页 | `src/app-shell.ts:79` |

退休路径（`src/router.ts:57-62`，`history.replaceState` 归一，不做 404）：

| 旧路径 | 目标 |
|---|---|
| `/settings` | `/` |
| `/gateway` | `/` |
| `/about` | `/` |
| `/router` | `/` |

未知路径（含已删除的 `/admin/*`）回退渲染工作台（`src/app-shell.ts:912-921`）。服务端 SPA fallback 与 Rust 单二进制资源内嵌保持不变，属于迁移的非改动面。

## 2. 页面/视图清单（19 个自定义元素）

「处置」为迁移意向，不是实现记录：**重写** = Lit/WA 结构整体替换为 React 组件；**复用** = 纯逻辑/数据适配层原样或轻改后继续使用；**删除** = 随旧栈退场的实现件。

| # | 元素 / 文件 | 职责 | 主要状态与入口 | 关联 capability | 处置 |
|---|---|---|---|---|---|
| 1 | `sebas-app` `src/app-shell.ts` | 应用壳：auth 探测门禁（checking/login/setup/ready）、rail 框架、rail 宽度拖拽、窄屏 Drawer、core reachability 锁定、设置弹窗与通知层挂载 | 鉴权四态；`inert` 锁定；`sebas:ws-state` | webui、agent-workbench、webui-user-management | 重写 |
| 2 | `sebas-dashboard` `src/views/dashboard.ts` | 工作台主区：项目头、转录、composer、审批/队列、归档只读视图、深链聚焦 | loading / error / empty / archive / terminated / stale-project / viewer-readonly | agent-workbench、session-*、permission-flow | 重写（纯投影函数复用） |
| 3 | `sebas-project-rail` `src/views/project-rail.ts` | 项目树、项目内会话、等待组、History、增加项目/目录选择器、移除/重命名/归档确认 | 空项目、展开态、`waiting`、history 开合、degraded 提示、错误内联 | project-session-actions、agent-workbench | 重写（持久化 adapter 复用） |
| 4 | `sebas-sessions` `src/views/sessions.ts` | `/sessions` 总览：创建表单、统计 chips、卡片、聚焦/关闭 | loading/error/empty、viewer 只读 | agent-workbench、webui-user-management | 重写 |
| 5 | `sebas-usage` `src/views/usage.ts` | 用量看数：来源/粒度/维度切换、汇总卡、SVG 图、不可达/空/局部警示 | day/hour、all/router/local、无数据、router 不可达 | usage-statistics | 重写（`windowTotals`/`seriesFor`/`detailRows` 等纯投影复用） |
| 6 | `sebas-settings-modal` `src/views/settings-modal.ts` | 设置工作区：通用/外观/服务/用户/模型/别名/Agent/技能/环境变量/关于，RBAC 裁剪 | 分区记忆、读写中/失败/成功、角色裁剪 | webui、provider-management、agent-settings、agent-skills、webui-user-management | 重写（分区导航与草稿语义保留） |
| 7 | `sebas-login` `src/views/login-view.ts` | 登录门禁 | 校验失败、提交中、成功回调 | webui（鉴权） | 重写 |
| 8 | `sebas-setup` `src/views/setup-view.ts` | 首启 root 建户 | 校验、409/失败、成功 | webui-user-management | 重写 |
| 9 | `sebas-transcript-view` `src/views/transcript-view.ts` | 转录：条目渲染、live tail、Markdown 结算、过程折叠、未读缝、滚动跟随、工具配对 | live/settled、空流、读缝、view-all 弹窗 | agent-workbench、live-turn-stream、fold-tool-calls | 重写（纯转换/游标复用） |
| 10 | `sebas-workbench-composer` `src/views/workbench-composer.ts` | 输入区：多行输入、提交/停止、排队、斜杠命令、模式下拉、模型芯片 | idle/turn/awaiting-receipt/parked/starting/failed/reachability/viewer | agent-workbench、core-session-channel、session-slash-commands、acp-model-selection | 重写 |
| 11 | `sebas-new-session-dialog` `src/views/new-session-dialog.ts` | 新建会话对话框（项目上下文、agent 必选、模型两级选择、mode） | 空目录、默认 agent、上次模型预选、失败内联 | agent-workbench、acp-model-selection | 重写（`lastUsedModelPair` 复用） |
| 12 | `sebas-review-cards` `src/components/review-card.ts` | 审批卡：读模型 + WS 幂等合并，allow-once/allow-session/deny | 并行请求、刷新重建、迟到拒绝 404、expired | permission-flow、agent-workbench | 重写 |
| 13 | `sebas-pending-stack` `src/components/pending-stack.ts` | 待执行提交栈：上移/移除、丢弃提示、繁忙提交 | staging/turn、rejection 两级反馈 | workbench-turn-queue、core-session-channel | 重写 |
| 14 | `sebas-folder-picker` `src/components/folder-picker.ts` | 目录树选择器（基于 `wa-tree`）+ 新建子目录 | 树展开、空路径、越界/非法名 | webui/projects、workspace-root | 重写（换 AntD Tree，API 不变） |
| 15 | `sebas-notice-layer` `src/components/notice-layer.ts` | 视口通知层：瞬时 toast 栈（上限 3）、ws down 横幅、fatal 横幅+遮罩 | info/warn/error、8s 去重、驻留 | webui（通知） | 重写（store 复用） |
| 16 | `sebas-notice-banner` `src/components/notice-banner.ts` | 持续态驻留横幅（多行 cause、不可关） | warn/fatal | webui（通知） | 重写 |
| 17 | `sebas-status-badge` `src/components/status-badge.ts` | 状态胶囊：label + slug + glyph（形状/文字/颜色三通道） | 7 相位 | agent-workbench | 重写（映射复用到 AntD Tag） |
| 18 | `sebas-line-chart` `src/components/line-chart.ts` | 手写 SVG 折线图 + 纯几何计算 | 多序列、tooltip、空数据 | usage-statistics | 组件重写、几何纯函数复用 |
| 19 | `sebas-model-aliases` `src/views/settings-aliases.ts` | 设置→别名分区 CRUD | 校验、失败内联、RBAC | router-model-aliases | 重写 |

非自定义元素但承载页面的模块：

| 模块 | 职责 | 处置 |
|---|---|---|
| `src/views/settings-aliases.ts` | 见上（挂入设置分区） | 重写 |
| `src/views/session-usage.ts` | 会话 token 芯片投影（「未上报」诚实态） | 复用纯函数 |
| `src/views/session-key-label.ts` | 编码 key → 「渠道 · 本地段」友好标签 | 复用 |
| `src/views/mode-vocabulary.ts` | 四模式单一词汇（标签/解释/徽章） | 复用（文案可能随蓝图调整） |
| `src/views/role-visibility.ts` | role→写入口可见性（呈现层裁剪，非防线） | 复用 |
| `src/views/skill-frontmatter.ts` | SKILL.md frontmatter 剥离 | 复用 |
| `src/views/turn-notify.ts` | 非聚焦会话回合终点通知接线 | 复用（改到 store 订阅） |
| `src/views/focus-events.ts` | `sebas:project-follow` 等焦点事件名 | 复用事件契约 |
| `src/views/split-persist.ts` | rail/composer 尺寸持久化与 clamp | 复用 adapter |
| `src/views/workbench-persist.ts` | 聚焦会话 key 持久化 | 复用 |
| `src/views/unread-cursor.ts` | 每会话已读水位 + 窗口广播 | 复用 |
| `src/branding.ts` | 品牌文案常量 | 复用或按蓝图改文案 |
| `src/theme.ts` | 三态主题、`wa-dark` 开关、pre-paint 语义 | 逻辑复用、DOM 开关换 AntD algorithm |
| `src/router.ts` | 极简 History API matcher / 退休重定向 / navigate | 复用为 Hook/Context |
| `src/notify.ts` | 分级通知模块级 store | 复用为框架无关 store |
| `src/components/markdown.ts` | `marked → DOMPurify → highlight.js` 消毒管线 | 复用 |
| `src/components/icons.ts` | 内联 SVG 图标集（装饰性 aria-hidden） | 复用数据，渲染改 React |
| `src/components/wa-hide-guard.ts` | WA dialog hide 来源守卫 | 删除（WA 专属 workaround） |
| `src/components/wa-select-rescue.ts` | WA select popover 残留清理 | 删除（WA 专属 workaround） |
| `src/styles/tokens.css` | 深/浅两套语义 token（status/signal/notice/宽度/字体/动效） | 值复用，映射到 AntD token + CSS variables |
| `src/styles/wa-overrides.css` | WA 组件覆盖 | 删除 |
| `src/styles/app.css`、`src/styles/shared.ts` | 全局与视图共享样式 | 重写为 CSS Modules / 全局基础样式 |
| `src/test-support/wa-polyfills.ts` | 测试期 WA polyfill | 删除 |

## 3. API 面（HTTP）

唯一入口 `src/api/client.ts`（1,607 行，`api` 对象 + `projects`），视图不直接 `fetch`。迁移不改变 wire 形状。

| 领域 | 端点 |
|---|---|
| 概览/会话读 | `GET /api/summary`、`GET /api/sessions`、`GET /api/sessions/{key}?entries_after=n`、`GET /api/sessions/{key}/approvals`、`GET /api/archive`、`GET /api/archive/{key}` |
| 会话写 | `POST /api/sessions`、`.../model`、`.../mode`、`.../label`、`.../message`、`.../cancel`、`.../pending/{id}/remove`、`.../pending/{id}/move`、`.../close`、`.../switch`、`.../activate`、`.../archive`、`.../restore`、`POST /api/permissions/{rid}/answer` |
| 项目 | `GET/POST /api/projects`、`.../{id}/remove`、`/api/projects/reorder`、`.../{id}/rename`、`GET .../{id}/branch` |
| 项目/会话选择器 | `GET /api/fs/browse`、`GET /api/fs/browse-dirs`、`POST /api/fs/mkdir` |
| 模型/provider | `GET/POST /api/providers`、`PUT/DELETE .../{name}`、`POST .../{name}/probe`、`GET /api/provider-defaults`、`GET /api/provider-presets`、`GET/POST /api/model-aliases`、`PUT/DELETE .../{alias}` |
| Agent | `GET/POST /api/agents`、`PUT/DELETE .../{id}`、`POST /api/agents/install`、`GET /api/nodes` |
| Skills | `GET /api/skills`、`GET .../{name}`、`DELETE .../{name}`、`POST /api/skills/sync` |
| 鉴权/用户 | `GET /api/auth/me`、`POST /api/auth/login`、`/setup`、`/logout`、`GET/POST /api/users`、`.../{id}/password`、`/role`、`/enabled`、`DELETE .../{id}` |
| 管理/用量 | `GET /api/admin/status`、`/events`、`/services`、`POST .../services/{name}/enable|disable|restart`、`POST /api/admin/update`、`/update/dry-run`、`/update/dev`、`/admin/rollback`、`GET /api/usage/timeseries?granularity=&days=&tz_offset=&source=` |
| 只读元信息 | `GET /api/about`、`GET /api/env` |

错误语义：`ApiError(status, code, count)` 与 `NetworkError`（`src/api/client.ts:885-960`），`errorText` 统一中文呈现；401 触发登录门、403 呈现拒绝、404/409/503 类型化处理。迁移必须保留真实错误呈现，不得伪造成功。

## 4. WS 面（`/ws` 单连接）

入口 `src/api/shared-ws.ts`（单例）→ `src/api/ws.ts`（532 行）。当前 `ws.ts` 顶部注释称实现为 **Lit ReactiveController**，迁移需拆除框架依赖，保留协议与重连语义。

- 请求：`core.reachability.get`（`src/app-shell.ts:718`）。
- 服务端事件：`session.created`、`session.updated`、`session.removed`、`session.pending_dropped`、`session.turn_stalled`、`session.resync`、`config.updated`、`permission.requested`、`turn.append`（带上限增量 `entries` + `seq`）、`core.reachability`（ok/kind/cause）。
- 协议：三帧封套（request/response/notification）、JSON codec、关联 id、超时与错误；重连退避、`MAX_UNOPENED_RECONNECTS` 有界收敛、重连后 `sebas:refetch`。
- 窗口级事件契约：`sebas:ws-state`、`sebas:refetch`、`sebas:anchor-advanced`、`sebas:rail-focus`、`sebas:project-follow`、`sebas:composer-focus-request`。React 版可改为 store 订阅，但语义等价性需在迁移矩阵中保留验证。

## 5. 浏览器持久化 key（localStorage）

| key | 值语义 | 读/写位置 | 迁移处置 |
|---|---|---|---|
| `sebas:theme` | `'system' \| 'dark' \| 'light'`，默认 system | `src/theme.ts:14`；`index.html` 内联 pre-paint | 保留 key 与取值；pre-paint 逻辑需在新 `index.html` 等价保留 |
| `sebas.rail-width` | 整数 px，clamp 180–520，默认 280 | `src/views/split-persist.ts:14`、`src/app-shell.ts:99` | 保留（AntD Layout 尺寸适配器读写） |
| `sebas.composer-height` | 整数 px，最小 120、上限主区一半，默认 220 | `src/views/split-persist.ts:16` | 保留 |
| `sebas.workbench-focus` | 聚焦会话 encoded key | `src/views/workbench-persist.ts:17` | 保留 |
| `sebas.rail-expanded` | 展开项目集合的序列化值 | `src/views/project-rail.ts:121` | 保留 |
| `sebas.rail-history-open` | `'1'`/`'0'`，缺省收起 | `src/views/project-rail.ts:128` | 保留 |
| `sebas:seen:<encodedKey>` | `{"anchor_count": n}`（u64，单调 max；旧 `seen_ts` 读作 fully-read，下次写入覆盖） | `src/views/unread-cursor.ts:24` | 保留 key 形态与旧值兼容语义 |
| `lastSettingsSection` | 设置分区 id | `src/views/settings-modal.ts:138` | 保留；分区 id 变更需做旧值回退 |
| `lastUsedModelPair` | `{provider, model}` JSON | `src/api/model-catalog.ts:118` | 保留 |

共同约束：所有访问必须防御隐私模式/opaque origin（抛错时退化为默认）；不得改写旧值含义；布局改版若需新偏好，新增 key。

## 6. 在途 change 交集

工作区只有本 change 未跟踪；以下 16 个未归档 change 触及 `sebas-webui/frontend/src` 或 `tests/testsuite-webui`。除 `add-webui-round7-gaps`、`fix-webui-qa-round3`、`fix-webui-qa-round7`、`webui-i18n-sweep` 各剩 1 条**全量回归勾选**外，其余任务均已勾选（代码已落在当前分支），因此它们主要是**测试选择器迁移的语义来源**，而不是仍在变动的实现面。

| change | 触及 capability | 与本 change 的交集 |
|---|---|---|
| `add-webui-round7-gaps` | agent-workbench、usage-statistics、webui | 会话用量芯片、连接徽标、目录选择器 mkdir、native/ACP 双通路旅程 |
| `fix-webui-qa-round3` | agent-workbench、usage-statistics、webui、webui-ws-rpc | 布局/对账/分区 IA/WS 鉴权旅程；其回归任务未勾 |
| `fix-webui-qa-round6` | agent-settings、agent-workbench、permission-flow、project-session-actions | 行名、设置交互、停止可达性 |
| `fix-webui-qa-round7` | acp-model-selection、agent-workbench、webui-ws-rpc | 长转录布局、未认证 WS；其回归任务未勾 |
| `fix-webui-qa-round8` | agent-workbench | 转录滚动、native 呈现、会话 key 友好化 |
| `fix-webui-qa-round9` | agent-workbench | 行重命名、项目重命名、设置交互 |
| `fix-webui-qa-round10` | acp-model-selection、agent-settings、agent-workbench、provider-management | 转录直播布局、rail 历史徽章、composer 模型拒绝、provider RBAC |
| `fix-webui-qa-round11` | agent-settings、agent-skills、agent-workbench、router-model-aliases、session-lifecycle、usage-statistics、webui | 技能 RBAC、设置分区、历史链接、回合终点通知 |
| `fix-webui-qa-round12` | agent-skills、agent-workbench、webui | 技能同步 RBAC、转录增量 |
| `fix-webui-qa-round13` | agent-workbench、usage-statistics、webui | rail/用量/设置细节 |
| `fix-webui-qa-round14` | agent-workbench、webui、webui-user-management | viewer 只读、历史计数、history 开合持久、RBAC 旅程 |
| `fix-parallel-approval-routing` | permission-flow | 并行审批路由语义 |
| `fold-tool-calls-into-process-tree` | agent-workbench、permission-flow | 工具 call id 配对折叠、native 工具块 |
| `gate-agent-directory-writes` | webui-user-management | agents 写入口角色裁剪与 403 执法 |
| `make-pi-ride-sebas-provider` / `make-pi-the-builtin-agent` | agent-workbench、pi-agent、webui、core-session-channel | agent 目录、创建对话框与 session 协议（前端只呈现，无新增 UI 面） |
| `add-agent-auto-install` | agent-settings、webui | 设置→Agent 一键安装入口 |
| `webui-i18n-sweep` | webui | 中文文案合同；其回归任务未勾，文案快照测试需随迁移重写而非删除 |

语义来源是 `openspec/specs/agent-workbench/spec.md`（56 个 requirement）与 `openspec/specs/testsuite-webui-browser/spec.md`（21 个 requirement / 55 个 scenario），不是这些 change 的临时 tasks。

## 7. 复用 / 重写 / 删除总表

**直接复用（框架无关，保留测试）**

- `src/api/client.ts`、`src/api/model-catalog.ts`、`src/api/ws.ts`、`src/api/shared-ws.ts`：协议、类型、目录与 WS 客户端。`ws.ts` 需去掉 Lit ReactiveController 外壳，其余不动。
- `src/router.ts`：matcher/redirect/navigate 纯函数。
- `src/notify.ts`：订阅式通知 store。
- `src/views/split-persist.ts`、`workbench-persist.ts`、`unread-cursor.ts`：localStorage adapter 与单调水位。
- `src/views/session-key-label.ts`、`mode-vocabulary.ts`、`role-visibility.ts`、`session-usage.ts`、`skill-frontmatter.ts`、`turn-notify.ts`、`focus-events.ts`：纯投影/词汇/事件契约。
- `src/components/markdown.ts`：消毒管线（仅换成受控 React 渲染器）。
- `src/components/line-chart.ts` 的几何纯函数。
- `src/components/icons.ts` 的 SVG path 数据。
- `src/styles/tokens.css` 的两套语义色/token 值（映射而非照搬）。

**重写（React + AntD）**

- 19 个 `sebas-*` 视图/组件的渲染、样式、生命周期与交互。
- `src/styles/app.css`、`src/styles/shared.ts` 的布局样式。
- 40 个 colocated 测试中依赖 Lit/WA/Shadow DOM/源码字符串的断言（`a11y.test.ts`、`i18n-copy.test.ts`、`wa-overrides.test.ts` 等）。
- `src/test-support/setup.ts`、`wa-polyfills.ts` 的测试环境。

**删除（旧栈专属）**

- `src/components/wa-hide-guard.ts`、`src/components/wa-select-rescue.ts`。
- `src/styles/wa-overrides.css`、`src/test-support/wa-polyfills.ts`。
- 依赖：`lit`、`@awesome.me/webawesome` 及其 polyfill/图标 base-path 逻辑。
- `preview.html` 旧预览入口（若仍被引用，随 5.3 一次切换删除）。

## 8. `sebas-*` 视图与 capability 覆盖核对

| 视图 | 覆盖 capability | 覆盖检查 |
|---|---|---|
| `sebas-app` | webui、agent-workbench | ✅ |
| `sebas-dashboard` | agent-workbench、session-lifecycle、live-turn-stream、permission-flow | ✅ |
| `sebas-project-rail` | project-session-actions、agent-workbench、state-store(projects) | ✅ |
| `sebas-sessions` | agent-workbench、webui-user-management | ✅ |
| `sebas-usage` | usage-statistics、add-local-usage-statistics | ✅ |
| `sebas-settings-modal` | webui、provider-management、agent-settings、agent-skills、webui-user-management、router-model-aliases、workspace-root | ✅ |
| `sebas-login` | webui（鉴权） | ✅ |
| `sebas-setup` | webui-user-management | ✅ |
| `sebas-transcript-view` | agent-workbench、live-turn-stream、fold-tool-calls-into-process-tree | ✅ |
| `sebas-workbench-composer` | agent-workbench、core-session-channel、session-slash-commands、acp-model-selection | ✅ |
| `sebas-new-session-dialog` | agent-workbench、acp-model-selection、project-session-actions | ✅ |
| `sebas-review-cards` | permission-flow、agent-workbench | ✅ |
| `sebas-pending-stack` | workbench-turn-queue、core-session-channel | ✅ |
| `sebas-folder-picker` | webui/projects、workspace-root | ✅ |
| `sebas-notice-layer` / `sebas-notice-banner` | webui（分级通知） | ✅ |
| `sebas-status-badge` | agent-workbench（状态单点呈现） | ✅ |
| `sebas-line-chart` | usage-statistics | ✅ |
| `sebas-model-aliases` | router-model-aliases | ✅ |

无未归属视图；`session-detail`、`admin`、独立 `router`、`about` 页面已在 IA v2 退休，不在迁移清单内。

## 9. 迁移风险与待决

1. **流式转录**：`transcript-view.ts`（2,880 行）+ 143 个测试是最大风险面；审批/工具/未读缝/滚动锚定共用同一游标语义，必须按条目隔离而不是整列表重渲染。
2. **设置工作区**：`settings-modal.ts`（4,699 行）拆为分区时，草稿、RBAC、写入失败与「无独立路由」四条语义都不能丢。
3. **WS 的 Lit 依赖**：`ws.ts` 的 ReactiveController 必须拆成框架无关订阅，但重连/鉴权/有界收敛行为须由既有 `ws.test.ts` 语义重写承接。
4. **localStorage 旧值**：`sebas:seen:*` 的旧 `seen_ts` 兼容与 `lastSettingsSection` 的非法值回退是「迁移即消失点」，不能顺手改格式。
5. **在途 change 的测试锚点**：多数 change 已实现未归档，迁移矩阵必须以 spec/COVERAGE 为准，逐条对应，避免把它们的任务级验收当成可删除用例。
6. **基线非全绿**：前端 `tsc --noEmit` 在干净基线已有 45 条错误（详见 `baseline-report.md`）；迁移不得把这些算作新版回归，也不应顺手修无关类型错误。
