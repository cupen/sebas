## MODIFIED Requirements

### Requirement: WebUI usage view

The WebUI SHALL provide a usage view (route `/usage`, reachable from the
sidebar) that renders the timeseries as a line chart: one line per model,
with a day/hour granularity toggle and a token dimension selector (total,
input, output, cache). The view SHALL show summary numbers for the current
window and SHALL distinguish the honest empty states: no usage data versus
router unreachable. The view SHALL require an authenticated session, like
other views.

图表坐标轴 SHALL NOT 呈现重复刻度：刻度生成 SHALL 对等值标签去重，相邻刻度
重复（QA round13 观察：y 轴呈现 1/1/1/0/0）视为呈现缺陷。

本视图的数据源口径 SHALL 钉死为 router 用量记录（`usage.db` 的 timeseries
聚合）：ACP 直连回合不经 router、不产生用量记录，其 token 计数呈现于会话
头部，SHALL NOT 计入本视图；视图 SHALL 以一行数据源说明向操作者交代该口径。
仅有 ACP 会话产生 token 的窗口 SHALL 如实呈现「无用量数据」空态，SHALL NOT
伪装成 router 故障，也 SHALL NOT 渲染无刻度的空图。

#### Scenario: day view renders per-model lines

- **WHEN** the usage view is open with day granularity and the aggregation
  reports two models over the window
- **THEN** the chart draws two distinguishable lines over the date axis,
  and the legend identifies each model

#### Scenario: granularity toggle switches the series

- **WHEN** the user switches the granularity toggle from day to hour
- **THEN** the chart re-renders with the 24 hourly buckets of today

#### Scenario: router unreachable empty state

- **WHEN** the aggregation request returns the router-unreachable cause
- **THEN** the view presents a router-unreachable notice (not a bare
  error), distinct from the no-data empty state shown when aggregation
  succeeds with all-zero buckets

#### Scenario: 相邻刻度不重复

- **WHEN** 窗口内数值很小（如最大值 ≤5）使默认刻度生成出等值相邻标签
- **THEN** y 轴实际呈现的刻度序列相邻互异（如 0/1/2/3/4 或 0/1），不存在
  相邻重复（1/1/1/0/0 形态绝迹）

#### Scenario: 纯 ACP 流量窗口呈现无数据空态

- **WHEN** 聚合窗口内只有 ACP 直连会话产生了 token（router 用量记录为零）
- **THEN** 视图呈现「无用量数据」空态，并带「数据来自 router 流量」口径
  说明行
- **AND** 视图不渲染空图、不呈现 router 不可达错误
