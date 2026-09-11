# Design: revamp-settings-nav-and-models-editor

## Context

单文件前端 `sebas-webui/frontend/src/views/settings-modal.ts`（约 2500 行 Lit 组件）承载全部六个分区；导航元数据集中在 `SECTIONS` 常量（settings-modal.ts:85），分区记忆在 localStorage `lastSettingsSection`（非法值已回退缺省，机制可复用）。fetch 链路 `api.fetchProviderModels(name)` 与 core op 不动——本变更只改前端呈现与状态归属。动机见 proposal.md。

## Goals / Non-Goals

**Goals:**
- 导航 IA 收敛为「偏好面（Generic/Appearance）— 管理面（Services/Models）— 信息面（About 压底）」三组。
- 原 Settings 总览拆解：只读项并入 About，维护动作删除。
- fetch 交互从「行内按钮 → 只读结果列表 → 逐条挑选」收敛为「编辑器内按钮 → 草稿整单替换 → 普通保存」。
- Models 区块文案与按钮精简。

**Non-Goals:**
- 多语言切换实现（Generic 只占位信息架构；i18n 字典另立变更）。
- 后端 API、core 抓取 op、配置 schema 不变。
- IM `/provider` 卡片的结果卡交互不变（provider-management spec 中保留）。

## Decisions

- **D1: Generic 分区内容 = 原 Env 只读表，语言切换仅占位**
  原 Settings 的三个只读项与 Env 表都是"通用杂项"，但性质不同：只读项是实例事实（去 About），Env 表是环境参考（留 Generic）。语言切换按钮本阶段不渲染——没有 i18n 字典渲染一个死按钮不如不渲染，spec 只承诺"预留信息架构位置"。替代方案（把 Env 表也塞 About）否掉：About 语义是"这个实例是什么"，Env 是"进程读什么变量"，混在一起 About 变垃圾抽屉。

- **D2: 分区记忆回退复用既有非法值机制**
  `readLastSection()` 已做"非法值回退 null → 缺省分区"。旧值 `settings` 天然落入该路径，无需迁移代码，只把缺省值常量从 `settings` 换成 `generic`。`Env` 旧记忆值同样处理（并入 Generic 后 `env` 不再是合法分区 id——为减少 memory key 兼容面，分区 id 直接改名为 `generic`，`env` 成为非法值回退）。

- **D3: fetch 状态机收敛进 editor 草稿**
  现状 `fetchResult` 是组件级 state，与 `editor` 并列，导致"编辑器外还有一份模型列表"。新模型：fetch 成功后直接 `setEditor({ models: deduped })`，复用编辑器既有的 dirty/save/cancel 语义；`fetchResult` 缩为 `fetchState: 'pending' | { error } | null` 仅存进程序与错误，成功结果不再单独呈现。同 id 保 tags 用 Map（id → entry）合并：以抓取顺序为骨架，旧 entry 的 tags 覆盖到新骨架同 id 上。替代方案（追加合并）否掉：语义应为"与官方同步"，追加残留过期 id。

- **D4: 行内 🔍 删除，编辑器按钮条件渲染**
  编辑器按钮仅在"provider 有可用 base URL"（preset 有 code-table URL 或 custom 任一槽位非空）时渲染；preset 编辑态用 presetDef 的 URL 判断。行内按钮删除后 provider 行只剩 ★/✎/🗑，视觉同步减负。

- **D5: 导航视觉参数写进样式而非主题令牌新增**
  36px 行高 / 0.875rem / 160px 栏宽 / accent 竖条（2px inset box-shadow 或 border-left）都是局部样式调整，不新增全局令牌，避免影响其他视图。About 压底用 `.nav { display: flex; flex-direction: column }` + About 项 `margin-top: auto`，分隔线为左右留白 12px 的 1px 低对比线。

- **D6: 删除重置 Settings 连带清理**
  `resetSettings()`、`resetSettingsOpen` state、确认 dialog、 Maintenance 区 CSS、restartAll 的调用与按钮一并移除；`loadOverview` 改名/迁移为 About 的 INSTANCE 段加载。

## Risks / Trade-offs

- [测试引用旧分区 id 与旧交互（settings-modal.test.ts 引用 `settings`/`env` 分区、fetch 挑选流）] → 随实现同步重写测试用例；既有测试即规格替身，改动面与 spec delta 一一对应。
- [用户习惯了 🔍 行内入口] → 可接受：入口只深一层（✎ 打开编辑器即见），且整单替换比挑选更省操作。
- [Generic 分区初期内容单薄（只有 Env 表）] → 接受过渡态；分区描述文案如实写"通用偏好与杂项"。

## Migration Plan

纯前端变更，无数据迁移。localStorage `lastSettingsSection` 旧值经 D2 回退。回滚 = revert commit。

## Open Questions

- 语言切换的真实实现（i18n 框架选型、字典组织）——留待后续变更，本设计不预设。
