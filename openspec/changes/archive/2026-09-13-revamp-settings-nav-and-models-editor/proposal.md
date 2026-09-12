# Proposal: revamp-settings-nav-and-models-editor

## Why

Settings 弹窗的导航与 Models 编辑器积累了一批体验问题：导航按钮过小难点击、分区顺序不合心智、首项「Settings」与弹窗同名语义含混、维护区混入低价值高危动作；Models 编辑器的模型条目区块文案啰嗦、fetch 按钮位置与「抓取-挑选」交互割裂。本变更做一轮以操作员体验为准的 IA 重排与交互收敛。

## What Changes

- **导航重排（BREAKING，前端交互语义）**：分区顺序改为 `Generic`（新，语言切换等通用偏好的家，占位先行）→ `Appearance` →〔分隔线〕`Services` → `Models` →〔弹性留白 + 分隔线，压底〕`About`。导航项加宽加高（约 14px 字号 / 36px 行高 / 160px 栏宽）、整行 hover、当前项左侧 accent 竖条。`Env` 分区并入 `Generic`。
- **原 `Settings` 分区拆解（BREAKING）**：三个只读总览项（workspace root、default agent kind、default provider/model）并入 `About`（INSTANCE 段在上、BUILD 段在下）；「全部进程重启」与「重置 Settings」两个维护动作删除。localStorage 分区记忆对非法/已删除值回退到 `Generic`。
- **Models 编辑器精简**：模型区块标签「Model entries」改为「Models」；两句提示语删除；「＋ Add model」文字按钮改为纯 `＋` 通栏按钮。
- **fetch 交互重做（BREAKING，取代 add-fetch-models 的抓取-挑选语义）**：fetch 入口从 provider 行内挪到编辑器「Models」区块标题旁；抓取结果直接整单替换编辑器中的模型列表（同 id 保留人工 capability tags，按 id 去重），保存与否由用户走普通编辑流决定；行内 🔍 按钮与「只读结果列表 + 逐条挑选」UI 删除。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`: 修改「设置弹窗分区与缺省首项」（重排、Generic、About 合并、删除维护动作）与「Fetch models from the provider's official base URL」（编辑器内整单替换，取代结果列表 + 逐条挑选）。
- `provider-management`: 修改「Model probing」——webui 面的探测呈现从「结果卡片 + 挑选」改为「编辑器草稿整单替换」；core 探测 op、单 URL 选择、错误脱敏语义不变。

## Non-goals

- 不实现多语言切换功能本身（Generic 分区只留信息架构位置，文案仍英文；i18n 另立变更）。
- 不动 core 的 providers 域抓取 op、后端 API、配置 schema。
- 不动 Services 分区的逐服务 enable/disable/restart 语义。
- 不做 preset URL 只读块、Protocol 下拉、rename map 等其他编辑器元素的重设计。

## Impact

- 代码：`sebas-webui/frontend/src/views/settings-modal.ts`（导航、Settings/About/Models 三分区渲染、fetch 状态机），`components/icons.ts`（可能补 icon），对应 `settings-modal.test.ts`。
- 规格：`openspec/specs/webui/spec.md`、`openspec/specs/provider-management/spec.md` 的上述 requirement delta。
- 兼容：无 API/存储破坏；localStorage `lastSettingsSection` 旧值（如 `settings`）按既有非法值回退逻辑落到新缺省分区。
