# fix-webui-qa-round11 Design

## Context

第十一轮 GUI 验收的 11 个缺陷分三组根因：

1. **native 生命周期元数据不回填**（B-1 + B-2 + O-1 同根）：native 体（进程内
   native 内核，`SEBAS_AGENT_ROUTER_URL` 装配）从不向共享元数据通道发布卡相位
   与标题。`SessionStatus::derive`（sebas-webui/src/models.rs）在 `Active` +
   无卡相位时诚实落 `Queued`——呈现层派生是单点且正确，坏的是输入恒空。composer
   的停止钮（agent-workbench「Submit control reflects submission and turn
   state」）同样依赖 turn-in-flight 真值，native 路径恒无 → 按钮不出现。
2. **门禁/通知接线缺口**（A-1、B-3、A-3+B-6）：round10 已把 `settings.manage`
   权限键接到 provider 变更面（rbac 矩阵未动），技能删除路由漏接；分级通知层
   已建成但回合终点事件未接入；SPA 的 WS 客户端在未认证态就发起连接并无限重试。
3. **呈现层小缺陷**（A-2、A-4、A-5、A-6、B-4、B-5）：各自局部，无交叉。

沙箱事实（验收时的可复现装配）：fake-claude ACP + native `test/*` 场景模型、
`[router] listen 127.0.0.1:8791`、auth 开 + 三角色账户。QA 脚本在
`C:/Users/cupen/AppData/Local/Temp/sebas-qa-r11/qa-b-scripts/`（Playwright 驱动，
regression 可参照）。

## Goals / Non-Goals

**Goals:**

- native 会话在所有观察面（rail 点、历史卡、计数、composer 控件）呈现与 ACP
  一致的生命周期真相
- 技能删除面服务端强制 + 前端隐藏双层门禁，与 provider 面同键
- 回合终点可达（非聚焦通知）、登录墙安静（WS 门禁）
- 六个呈现缺陷逐一对齐对应 spec 场景

**Non-Goals:**

- 不改 rbac.rs 权限矩阵、不改 wire 形状、不动 `SessionStatus::derive` 的语义
- O-2（bash Windows 平台限制）、O-3（parallel 卡形态）、O-4（usage 口径）、
  O-5（「飞书 ·」前缀标签）按 proposal Non-goals 记录不修

## Decisions

- **D1 native 元数据回填走既有通道，不新增第二派生路径**：native 体在回合开始/
  结束/失败时向会话映射发布卡相位（`OnIt`/`Done`/`CrossMark`）与标题写入，让
  `SessionStatus::derive` 与 composer 的 turn-in-flight 判定继续吃同一输入。
  备选「前端按 native 事件单独推状态」被否：制造第二份派生逻辑，rail/历史/
  计数/composer 四个观察面要各自对齐，正是本轮缺陷的成因。根因嫌疑区在
  `src/agent_backend.rs` 的 native 装配面（ACP 路径有回填而 native 没有），
  实现时先以一条 native 回合的通道观测定位确切断点。
- **D2 技能删除门禁复用 `settings.manage`**：服务端路由守卫 + 前端
  role-visibility 隐藏删除控件，错误呈现样式与项目注册的「权限不足」内联红字
  一致。备选「新增 skills.manage 键」被否：round10 已确立 store 变更面 =
  `settings.manage` 的口径，技能仓删除同属 store 变更，矩阵不动。
- **D3 通知只补「非聚焦」分支**：回合完成/失败事件在会话非聚焦时发 info/error
  toast；聚焦中不弹（转录即呈现）。去重窗口沿用通知层既有语义，防止多回合连发
  刷屏。备选「所有回合都通知」被否：聚焦会话的终点转录可见，重复通知是噪音。
- **D4 WS 门禁在 SPA 侧收口**：认证态（`/api/auth/me` 结果 + 登录/登出事件）
  驱动连接生命周期——未认证不建连、认证失效拒绝后不重试、登录成功再建连；
  `auth = false` 部署维持「启动即建连」。服务端「鉴权与连接姿态不变」
  （webui-ws-rpc）不动。备选「服务端放行升级后立刻关」被否：仍然每次重连都打
  一次失败日志，且给未认证者暴露升级面。
- **D5 B-5 止停轮询在数据层而非呈现层**：会话加载失败（不存在/已关闭）后把该
  会话标记为不可得，后续周期同步跳过它；呈现保持现有「会话不可得」居中态。
  备选「仅前端停表」被否：周期同步（incremental sync）仍会再次打入。
- **D6 呈现小修各自最小化**：B-4 气泡容器 `white-space: pre-wrap`（含恢复后的
  转录渲染同一容器）；A-4 弹窗打开期间窗口级 keydown（capture）响应 Esc；
  A-5 图表右缘 inset；A-2 空下拉禁用态 + 指引文案（指向 Models 分区）；
  A-6 文案改写为完整短句。

## Risks / Trade-offs

- [native 卡相位回填改动核心会话元数据面] → 先在通道观测层确认断点再动手；
  回归用 `tests/testsuite_e2e_test.rs` 既有 native 旅程 + 新增状态推进断言；
  ACP 路径行为零改动（同一函数、输入来源不同）
- [门禁收紧可能影响依赖技能删除的既有流程] → 删除入口 CLI 不变（CLI 本就不经
  webui 角色门），只收 webui 面；root/admin 不受影响
- [WS 门禁改变连接时序] → `auth = false` 与已登录两条路径均有既有旅程覆盖；
  登录页新姿态补前端单测
- [通知接入点若落在 core 事件广播处可能对 IM 通道重复] → 只在 webui 面订阅
  既有事件接线，不加发新 wire 事件

## Migration Plan

纯行为修复与门禁收紧，无数据迁移。回滚 = revert 单个 commit 序列。

## Open Questions

（无——根因定位中的「确切断点在 native 装配面哪一行」按 D1 在实现期以观测
确定，不影响 spec 与任务拆分。）
