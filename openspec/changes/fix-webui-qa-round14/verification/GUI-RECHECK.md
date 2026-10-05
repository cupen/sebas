# GUI 复验记录（task 5.3，主 agent 真浏览器）

- 日期：2026-10-05；平台：ZCode IAB（WebKit 引擎）+ 仓库 Playwright chromium 套件（3c）
- 沙箱：core@9877（auth=on，admin/root + 四角色）+ router@8787（--debug），HOME/SEBAS_HOME 钉沙箱
- 截图：沙箱 `reports/img/r14-*.png`（capacity-rejection 为关键证据）

## 逐项复验结论

| 项 | 结论 | 证据 |
|---|---|---|
| 容量拒绝呈现（D-3-1/D-4-3，含服务端半边修复） | **PASS** | 引擎映射满 32 时 GUI 提交创建：弹窗内联红字「会话数已达上限 32」+ notice 同文案 + **URL 不变**（无幻影跳转）+ 弹窗保持；截图 `r14-capacity-rejection.png` |
| 幻影 URL 服务端根因 | **FIXED** | 修复前实测：POST 201+key 但 session_map 无行、详情 404、core 日志 `web_create_placeholder: begin_spawn_with failed e=Capacity(32)`；修复后引擎拒绝上抛（`capacity_rejection_propagates_instead_of_a_phantom_key` 单测钉住） |
| 预命名（4.7） | **PASS** | IAB 程序化置值链路：行名即现（「r14-预命名G」）；真实键入的 chromium 证据由 3c `qa-round14.spec.ts` 旅程钉住 |
| 别名消费（D-4-1） | **PASS** | composer 菜单「别名」组置顶、条目带徽标「别名 → sandbox-anthropic · claude-opus-4-6」；选中后 chip=opus-demo、转录落「模型已切换」条目 |
| 别名目标 provider 下拉（D-4-2，语义修正） | **PASS（修订后）** | 种子行「anthropic（config 种子·不可选）」列出但禁选带原因；store provider 别名创建持久化（`model_aliases` 载荷可见） |
| 历史计数（4.5） | **PASS** | 侧栏「历史 44」= 归档总数 |
| viewer /sessions 只读（D-5-1） | **PASS** | qa-viewer 登录后 /sessions 呈「只读总览」、无新建表单与写按钮 |
| viewer 只读视图（D-5-2） | **PASS** | viewer 工作台呈现只读 composer 说明；无 switch 请求的请求级断言由 3c RBAC 旅程（chromium）钉住 |
| 聚焦会话详情 | **PASS** | 正常创建的会话头部/转录/composer 全正常；「会话不可得」卡仅在容量 bug 的幻影 key 下出现（已修复） |

## IAB 限制与观察（非本 change 缺陷）

- **IAB 截图合成怪癖**：rail 偶发在截图中空白而 DOM 几何/样式正常（chromium 渲染正常，Playwright 套件佐证）。
- **WebKit composed-input 系统性缺口（记为遗留项）**：wa-input 的 `@input` 绑定依赖 composed input 事件穿出影子根——Chromium 成立、WebKit 不成立。影响所有 `wa-input @input` 绑定（登录、项目路径、设置表单……），非 round14 引入；4.7 标题已按「确认时刻直读 value」局部加固。**建议另立项**：统一改绑 Web Awesome 文档事件或直读 value。
- **部署隐患**：`dist/assets/index-*.js` 构建产物名不随内容变化（修复前后同名不同内容），HTTP 缓存会钉住旧包——建议核查 vite/rolldown 的 hash 配置。
- 设置里新建别名后，已挂载 composer 的模型菜单不热更新（重载后生效）——可用性观察项，未计缺陷。

## 复验期间发现并当场修复的实现缺口（3c 未覆盖面）

1. **服务端半边（P1 级缺口）**：`DispatchHandle::web_spawn` / `web_create_placeholder` 的 Err 臂吞掉 `Capacity` 后仍返回新 key → webui 201 假成功 + 幻影 URL。修复：拒绝原样上抛（`Result<ChannelKey, DispatchError>`），`session_backend` 三调用点映射为 `SessionRejection`（`rejection_from_dispatch`），`create_session` 处理器既有 `rejection_response` 接管。新增引擎单测 1 条。
2. **4.7 WebKit 加固**：确认时刻直读输入框实时值（不依赖 composed 事件的状态绑定）。
3. **D-4-2 语义修正**：种子 provider 行「列出但禁选+原因」（后端 store-only 校验维持——外键约束即域规则），delta 场景随修。
