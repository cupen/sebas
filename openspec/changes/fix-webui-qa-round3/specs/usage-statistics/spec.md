## MODIFIED Requirements

### Requirement: WebUI usage view

The WebUI SHALL provide a usage view (route `/usage`, reachable from the
sidebar) that renders the timeseries as a line chart: one line per model,
with a day/hour granularity toggle and a token dimension selector (total,
input, output, cache). The view SHALL show summary numbers for the current
window — the summary cards SHALL aggregate exactly the window the chart is
showing (granularity + time range), and SHALL update when the operator
switches granularity or range（不得常驻全量口径与图表口径不一致）. The view
SHALL distinguish the honest empty states: no usage data versus router
unreachable. All controls of the view (toggle, selector, refresh) SHALL fit
inside the view container at a 1280px viewport without clipping. The view
SHALL require an authenticated session, like other views.

#### Scenario: day view renders per-model lines

- **WHEN** the usage view is open with day granularity and the aggregation
  reports two models over the window
- **THEN** the chart draws two distinguishable lines over the date axis,
  and the legend identifies each model

#### Scenario: granularity toggle switches the series

- **WHEN** the user switches the granularity toggle from day to hour
- **THEN** the chart re-renders with the 24 hourly buckets of today

#### Scenario: summary cards follow the selected window

- **WHEN** the operator switches granularity or time range（如从近 14 天切到今天按小时）
- **THEN** 汇总卡数字与图表窗口一致地重算（今日无数据的窗口下请求数等卡片反映该窗口，而非全量历史）

#### Scenario: router unreachable empty state

- **WHEN** the aggregation request returns the router-unreachable cause
- **THEN** the view presents a router-unreachable notice (not a bare
  error), distinct from the no-data empty state shown when aggregation
  succeeds with all-zero buckets

#### Scenario: controls stay inside the container

- **WHEN** 1280px 宽视口打开 usage 视图
- **THEN** 刷新按钮等控件完整落在视口/容器内，无右缘溢出截断
