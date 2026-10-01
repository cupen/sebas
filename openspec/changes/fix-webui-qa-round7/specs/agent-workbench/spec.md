## ADDED Requirements

### Requirement: 转录任意内容不破坏布局

会话转录区 SHALL 在任意合法输入（含无空格超长 token、GFM 表格、长作者名/长 receipt 文本）下保持布局完整：转录滚动容器的横向 scrollWidth SHALL NOT 超出其视口宽度，用户消息与 agent 回复 SHALL 始终落在可视区内，且侧栏与主面板的相对位置 SHALL 不因单条内容而漂移。该约束 SHALL 在主题切换等重渲染后依然成立。

正文换行 SHALL 采用参与固有宽度收缩的折行策略（`overflow-wrap: anywhere` 级别），而非仅依赖不参与 min-content 计算的 `break-word`；markdown 管线产出的 `<table>` SHALL 以块级横向滚动呈现（不撑破父容器）；meta 行中的作者名等弹性文本 SHALL 具备收缩守卫（min-width:0 + 省略）。

#### Scenario: 无空格超长文本不撑破面板

- **WHEN** 操作者发送一条 2000+ 字符的无空格消息（如连续 `x`）
- **THEN** 转录滚动容器的 scrollWidth 与视口宽度同量级，用户气泡完整可见
- **AND** 切换主题或离开再回到该会话后，布局仍不横向溢出，横向滚动位置不漂移

#### Scenario: GFM 表格横向滚动

- **WHEN** agent 回复包含一张宽于容器的 markdown 表格
- **THEN** 表格以块级横向滚动呈现，转录容器本体不产生横向溢出

#### Scenario: 长作者名不挤占时间戳

- **WHEN** 消息 meta 行的作者名过长（或 receipt 文本过长）
- **THEN** 作者名收缩并省略，时间戳与回执芯片保持原位不被推出
