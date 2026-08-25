## Purpose

gateway 的可观测性出口：标准 Prometheus 抓取面（`/metrics`）供外部监控系统采集，JSON 摘要（`/admin/stats`）供 webui 数字卡片渲染。

## Requirements

### Requirement: Prometheus exposition

`GET /metrics` SHALL emit Prometheus text-format metrics, behind the admin
authentication layer. Metric families: `gateway_requests_total`
(counter, labels `provider`, `model`, `protocol`, `status`) counting every
request that completes at the proxy handler with its final status (upstream
status or gateway-generated error status); `gateway_request_duration_seconds`
(histogram with fixed buckets, same labels minus `status`);
`gateway_tokens_total` (counter, labels `provider`, `type` in
`input|output|cache_read|cache_creation`); `gateway_rate_limited_total`
(counter); `gateway_upstream_errors_total` (counter, label `provider`);
`gateway_active_requests` (gauge); `gateway_start_time_seconds` (process
start time). Metrics MUST NOT contain downstream token or key material,
request headers, or request bodies.

#### Scenario: scrape with auth

- **WHEN** a scraper presents the admin bearer token to `GET /metrics`
- **THEN** the response is text-format metrics including
  `gateway_requests_total` series

#### Scenario: metrics omit secrets

- **WHEN** a request carrying a downstream token is proxied and metrics are
  scraped
- **THEN** no series or label contains the token value

### Requirement: JSON stats summary

`GET /admin/stats` SHALL return a JSON summary for webui rendering:
process uptime; totals since start (request count, input/output/cache
tokens, rate-limited count, upstream error count); per-provider aggregates
(request count, error count, input/output tokens, average latency in
milliseconds); and the last configuration reload status with error text
when the last reload failed. Provider names are configuration items and may
appear; key material never does.

#### Scenario: stats reflect traffic

- **WHEN** three requests have been proxied to provider `alpha` and
  `GET /admin/stats` is called
- **THEN** the `alpha` entry reports request count 3

#### Scenario: stats surface reload error

- **WHEN** the last external reload attempt failed due to invalid JSON in
  the provider overlay file
- **THEN** the stats response reports the reload as failed with an error
  message
