# usage-statistics Specification

## Purpose

usage 的时序聚合查询面：把 usage.db 里逐请求的 token 消耗记录聚合成按天/按小时的分模型统计，经 core 反代到达 WebUI，以折线图呈现。router 仍是 usage 数据的唯一所有者与聚合者；WebUI 仍只连 core。

## Requirements

### Requirement: Router timeseries aggregation endpoint

router SHALL expose `GET /admin/usage/timeseries` behind the existing admin
authentication layer, accepting `granularity` (`day` | `hour`), `days`
(1–30, default 14, day granularity only), and `tz_offset` (minutes east of
UTC). The response SHALL aggregate the persisted usage records grouped by
model within time buckets: each bucket SHALL report per-model sums of
input, output, cache_read, and cache_creation tokens, plus request count.
Records whose token counts were not observed (null) SHALL contribute
nothing to token sums. Buckets SHALL cover the full requested window
zero-filled: day granularity returns every date in the window, hour
granularity returns hours 0–23 of the current day. The response SHALL NOT
contain key material or request content.

#### Scenario: two models aggregate into separate day buckets

- **WHEN** usage records exist for models `claude-sonnet` (input 10,
  output 50) and `gpt-4o-mini` (input 5, output 8) on the same day, and
  `GET /admin/usage/timeseries?granularity=day` is called
- **THEN** that date's bucket reports both models with their own token
  sums, and every other date in the window appears zero-filled

#### Scenario: unobserved tokens contribute nothing

- **WHEN** a record with null token counts (upstream error response) falls
  in the window
- **THEN** its bucket's token sums are unchanged, and its request count
  includes the record

#### Scenario: endpoint requires admin auth

- **WHEN** `GET /admin/usage/timeseries` is called from a non-loopback
  address without the admin bearer token
- **THEN** the response is 401, identical to other `/admin/*` endpoints

### Requirement: Timezone-aware bucketing

Day and hour bucket boundaries SHALL be computed in the timezone offset
carried by the request (`tz_offset`), not in UTC: a record logged at UTC
23:00 SHALL land in the next calendar day's bucket for an offset of
+8 hours.

#### Scenario: UTC 23:00 record lands on the next day for UTC+8

- **WHEN** a record is logged at `2026-09-29T23:00:00+00:00` and the
  timeseries is queried with `tz_offset=480`
- **THEN** the record's tokens appear in the bucket for `2026-09-30`

#### Scenario: default offset is UTC

- **WHEN** `tz_offset` is omitted
- **THEN** buckets are computed as if `tz_offset=0`

### Requirement: Hour granularity covers only the current day

Hour granularity SHALL aggregate only records from the current calendar
day computed in the requested offset, across hours 0–23. Once the day has
rolled over, an hour query SHALL NOT include previous days' hourly detail;
earlier days are only available through day granularity.

#### Scenario: hour query ignores yesterday

- **WHEN** records exist yesterday and today, and
  `GET /admin/usage/timeseries?granularity=hour&tz_offset=480` is called
- **THEN** the response contains exactly 24 buckets for today (in the
  requested offset) and no data from yesterday

### Requirement: Core usage proxy endpoint

core SHALL expose `GET /api/usage/timeseries` for authenticated WebUI
sessions (all roles), forwarding the query parameters to the router's
aggregation endpoint over loopback HTTP with the admin bearer credential,
and returning the aggregation response unchanged. The proxy SHALL resolve
the router address from configuration and SHALL tolerate router absence.

#### Scenario: authenticated request returns aggregation

- **WHEN** a logged-in WebUI session calls `GET /api/usage/timeseries?granularity=day&tz_offset=480`
  while the router is running
- **THEN** the response carries the router's aggregation payload

#### Scenario: unauthenticated request is rejected

- **WHEN** an unauthenticated client calls `/api/usage/timeseries`
- **THEN** the response is the same auth rejection as other `/api/*`
  endpoints

#### Scenario: router unreachable reports a cause

- **WHEN** the router process is not running and
  `GET /api/usage/timeseries` is called
- **THEN** the response is a structured error naming the cause (router
  unreachable), returned promptly without hanging, and other `/api/*`
  endpoints are unaffected

### Requirement: WebUI usage view

The WebUI SHALL provide a usage view (route `/usage`, reachable from the
sidebar) that renders the timeseries as a line chart: one line per model,
with a day/hour granularity toggle and a token dimension selector (total,
input, output, cache). The view SHALL show summary numbers for the current
window and SHALL distinguish the honest empty states: no usage data versus
router unreachable. The view SHALL require an authenticated session, like
other views.

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
