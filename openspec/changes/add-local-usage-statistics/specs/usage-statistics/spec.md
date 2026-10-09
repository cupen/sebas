## MODIFIED Requirements

### Requirement: Core usage proxy endpoint

core SHALL expose `GET /api/usage/timeseries` for authenticated WebUI
sessions (all roles) with a `source` parameter (`router` | `local` |
`all`, default `all`). For `source=router` core SHALL forward the query
parameters to the router's aggregation endpoint over loopback HTTP with
the admin bearer credential and return the aggregation response
unchanged, resolving the router address from configuration and tolerating
router absence as before. For `source=local` core SHALL aggregate its
local usage store directly. For `source=all` core SHALL aggregate the
local store AND, when the router is reachable, merge the router
aggregation into the same bucket structure (per-model rows carry
per-source subtotals); when the router is unreachable the response SHALL
still succeed with the local contribution intact and SHALL carry a
structured router-unreachable cause so clients can flag the missing
source honestly.

#### Scenario: authenticated request returns aggregation

- **WHEN** a logged-in session calls `GET /api/usage/timeseries?source=router`
  while the router is running
- **THEN** the response carries the router's aggregation payload unchanged

#### Scenario: router unreachable reports a cause

- **WHEN** `source=router` is requested explicitly while the router
  process is not running
- **THEN** the response is a structured error naming the cause, returned
  promptly without hanging, and other `/api/*` endpoints are unaffected

#### Scenario: source=local aggregates without the router

- **WHEN** `source=local` is requested while the router is not running
- **THEN** the response is a complete aggregation of the local store with
  no error

#### Scenario: source=all merges both sources with per-source subtotals

- **WHEN** `source=all` is requested and both the local store and the
  router store hold rows for the same day and model
- **THEN** that bucket's model row reports the combined sums plus
  per-source subtotals, and the same request is never counted twice

#### Scenario: source=all tolerates router absence

- **WHEN** `source=all` is requested while the router is unreachable
- **THEN** the response succeeds with the local contribution intact and
  carries the structured router-unreachable cause

#### Scenario: unauthenticated request is rejected

- **WHEN** an unauthenticated client calls `/api/usage/timeseries`
- **THEN** the response is the same auth rejection as other `/api/*`
  endpoints

### Requirement: WebUI usage view

The WebUI SHALL provide a usage view (route `/usage`, reachable from the
sidebar) that renders the timeseries as a line chart: one line per model,
with a day/hour granularity toggle, a token dimension selector (total,
input, output, cache), and a source selector (all, router, local). The
view SHALL show summary numbers for the current window and SHALL
distinguish the honest empty states: no usage data versus router
unreachable; with `source=all` a router-unreachable cause SHALL be shown
as a partial-data notice while local data still renders. The view SHALL
require an authenticated session, like other views.

#### Scenario: day view renders per-model lines

- **WHEN** the usage view is open with day granularity and the aggregation
  reports two models over the window
- **THEN** the chart draws two distinguishable lines over the date axis,
  and the legend identifies each model

#### Scenario: granularity toggle switches the series

- **WHEN** the user switches the granularity toggle from day to hour
- **THEN** the chart re-renders with the 24 hourly buckets of today

#### Scenario: source selector switches the aggregation

- **WHEN** the user switches the source selector from all to local
- **THEN** the chart re-renders from the local-source aggregation only

#### Scenario: router unreachable empty state

- **WHEN** the aggregation request returns the router-unreachable cause
  with `source=router`
- **THEN** the view presents a router-unreachable notice (not a bare
  error), distinct from the no-data empty state shown when aggregation
  succeeds with all-zero buckets

#### Scenario: partial data notice under source=all

- **WHEN** the view is on `source=all` and the response carries the
  router-unreachable cause with local data present
- **THEN** the chart still renders the local data and the view shows a
  partial-data notice naming the missing router source
