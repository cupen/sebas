## MODIFIED Requirements

### Requirement: WebUI usage view

The WebUI SHALL provide a usage view (route `/usage`, reachable from the
sidebar) that renders the timeseries as a line chart: one line per model,
with a day/hour granularity toggle and a token dimension selector (total,
input, output, cache). The view SHALL show summary numbers for the current
window and SHALL distinguish the honest empty states: no usage data versus
router unreachable. The view SHALL require an authenticated session, like
other views. The chart SHALL reserve enough inset for its axes: the rightmost
x-axis label SHALL be fully visible (not clipped by the chart edge) at every
supported granularity.

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
  simply has no records

#### Scenario: rightmost axis label fully visible

- **WHEN** the chart renders with data at either granularity
- **THEN** the rightmost x-axis label (e.g. the latest date) is displayed in full, not truncated at the chart edge
