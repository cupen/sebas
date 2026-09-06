# Design — add-agent-defaults-catalog

## Context

provider 管理已由上游 `refactor-provider-data-model` 落定为：真相源 = router 持有的
`providers.json`，管理面 = router admin API（`/admin/providers*`、`/admin/presets`），
WebUI 经 BFF（`/router/api/*`，控制秘密）代理。被 drop 的 `add-webui-model-config`
曾用「核心通道 additive op 写状态库」实现 provider 管理与 backend catalog，本 change
把其中**上游仍缺**的两项能力（agent defaults、pre-session catalog 选择器）在新架构上
重建。被 drop 版本的设计（通道 op、状态库真相）不复活。

## Goals / Non-Goals

**Goals:**
- defaults（默认 provider + model）可设置、可持久化、可清除，重启后保持。
- composer 模型选择器在**无任何会话**时即可提供模型选项（来自 defaults 指向 provider
  的 catalog），会话存在时保持既有会话级数据源。
- provider 管理页内完成"设默认"动作。

**Non-Goals:**
- 不改 provider 数据模型/真相源；不做会话级 override；不复活状态库写路径（见 proposal）。

## Decisions

### D1 defaults 的真相源放 router 侧，与 provider store 同域

- 存储与读写面：router admin 新增 `GET/PUT /admin/defaults`，持久化与 providers.json
  同域（router 状态）；WebUI BFF 以 `GET/PUT /api/agent-defaults` 代理。
- 备选否决：放状态库（被 drop 方案的路线）——与 provider 真相源分离，且 detached 形态
  下 provider 页本就依赖 router 可达，defaults 没有理由走另一条通道。
- 备选否决：塞进 providers.json 同文件——defaults 不是 provider，混存会让两个写者
  （CRUD 与 defaults）在同一文件上互相踩，独立键位更干净。

### D2 composer 选择器数据源优先级：会话 > catalog > 诚实不可用

- 有会话：保持现状（会话执行体的 `available_models`，ACP agent 声明优先）——
  不破坏 acp-model-selection 既有语义。
- 无会话：取 defaults 指向 provider 的 catalog。catalog 来源与 provider 管理页一致：
  自定义 provider 用 probe 落盘的模型列表，preset 派生跟随代码表（`/admin/presets`）。
- 两者皆无：显式"不可用"提示，不伪造空选项。
- defaults 未设置时不报错——直接落"无会话且无 catalog"分支（现状行为）。

### D3 设默认动作放 provider 管理页而非 composer

- 页内对 provider 行提供"设为默认"（及清除），展示当前默认；composer 只**消费**默认。
- 备选否决：composer 内联设默认——composer 的职责是发起会话，默认值是 provider 域的
  配置，收敛在管理页一处修改入口。

## Risks / Trade-offs

- [router 不可达时 composer 无会话场景失去模型选项] → 与 provider 管理页同 degraded
  姿态（显式不可用）；会话存在时的既有数据源不受影响。
- [defaults 与 provider 被删除的竞态] → 删除默认 provider 时 router 侧一并清除
  defaults（同写者域内完成），composer 端下次读取得到"未设置"。

## Migration Plan

纯新增面：defaults 未设置时所有行为与现状一致，无迁移。回滚 = revert。

## Open Questions

（无）
