# router-auth-rate-limit Specification

## Purpose
Defines the router's control plane: downstream API key authentication, the
per-key rate limiter, asynchronous usage accounting (JSONL sink, SSE and
buffered-body token extraction), and the access log — everything that decides
who may talk to the router and what gets recorded, as opposed to the request
forwarding itself.

## Requirements

### Requirement: Downstream key authentication

The router SHALL authenticate requests with a static key table built from
the `[router] auth_token` config (accepting either a single string or an
array). The key is extracted preferring `Authorization: Bearer <key>` over
`x-api-key: <key>`. A missing or unknown key yields `401` with message
`invalid or missing API key`, rendered in the protocol-appropriate error
shape for the sniffed protocol; the response SHALL never echo any key
material. `GET /healthz` is exempt from authentication.

#### Scenario: bearer preferred

- **WHEN** a request carries both a valid `Authorization: Bearer k1` and an
  invalid `x-api-key: k2`
- **THEN** authentication succeeds with key `k1`

#### Scenario: unknown key rejected

- **WHEN** a request presents `x-api-key: nope` against a table of `k1`
- **THEN** the response is 401 with body type `authentication_error`
  (Anthropic-sniffed) or `invalid_request_error` (OpenAI-sniffed) and
  contains none of the configured keys

### Requirement: Open router when unconfigured

When no `auth_token` is configured (or the router runs in `--debug` mode),
the router SHALL skip authentication entirely and serve requests without a
key; such requests are attributed to the shared `anonymous` identity.

#### Scenario: no auth_token configured

- **WHEN** the router config has no `auth_token` and a keyless request
  arrives
- **THEN** the request proceeds to routing and rate limiting without a 401

### Requirement: Per-key token-bucket rate limiting

The router SHALL rate-limit per extracted key (anonymous bucket for keyless
requests on an open router) using a token bucket: capacity tokens, refilled
at `refill_per_sec`, starting full, with lazy refill capped at capacity.
Configuration under `[router.rate_limit]` accepts either `capacity` +
`refill_per_sec`, or the `rpm` shorthand (capacity = rpm, refill = rpm/60);
the limiter is enabled iff `capacity` or `rpm` is set, and disabled (or in
debug mode) requests pass through at zero cost. Exceeding the bucket yields
`429` with message `rate limit exceeded` and error type `rate_limit_error`
in the protocol-sniffed shape. Buckets are independent per key.

#### Scenario: over capacity rejected

- **WHEN** a key with capacity 5 sends 6 requests in quick succession
- **THEN** the 6th response is 429 with type `rate_limit_error`

#### Scenario: independent buckets

- **WHEN** key `k1` exhausts its bucket and key `k2` then sends a request
- **THEN** `k2`'s request is not rate-limited

#### Scenario: rpm shorthand

- **WHEN** `[router.rate_limit] rpm = 60` is configured
- **THEN** the bucket holds 60 tokens refilling at 1 token per second

### Requirement: Usage record pipeline

Usage records SHALL be written by an asynchronous sink: the request path
pushes records onto a bounded channel (capacity 256) consumed by a background
writer that appends JSONL to `~/.sebas/router-usage.jsonl` (parent directory
created on demand). When the channel is full or the sink is closed, records
are dropped with a warning and the in-flight response is never blocked or
failed by usage accounting.

#### Scenario: sink overflow drops records

- **WHEN** more than 256 records are queued while the writer is stalled
- **THEN** excess records are dropped (warn-logged) and the corresponding
  responses were already served unaffected

#### Scenario: jsonl append

- **WHEN** two requests complete
- **THEN** the usage file contains two appended JSON lines under the
  router data directory

### Requirement: Usage record content

Each JSONL record SHALL carry: timestamp, protocol, model, provider,
upstream_model, status, latency_ms, ttft_ms, token counts (input, output,
cache_read, cache_creation), and an error field. The `key` field SHALL always
be written empty — records are not attributable to a downstream key. The
error field is populated only for router-side failures (e.g. upstream
connect failure → status 502); upstream 4xx/5xx responses record their
status with no router error.

#### Scenario: key never recorded

- **WHEN** a request authenticated as `k1` completes
- **THEN** the usage record's `key` field is the empty string

#### Scenario: upstream error recorded without router error

- **WHEN** the upstream returns 429
- **THEN** the record has status 429 and an empty error field

### Requirement: SSE usage tee

For `text/event-stream` responses the router SHALL extract usage
incrementally by feeding each forwarded chunk to a tolerant parser while the
bytes pass through unmodified. Extraction merges partial events across chunk
boundaries: Anthropic input and cache tokens from `message_start`, output
tokens from `message_delta`; OpenAI chat from `prompt_tokens`/
`completion_tokens` and Responses-API from `input_tokens`/`output_tokens`.
Malformed or truncated frames are skipped silently; the record settles with
best-effort (possibly null) usage on stream end or client disconnect, never
altering the byte stream. TTFT is captured from the first SSE chunk only.

#### Scenario: anthropic usage across chunks

- **WHEN** an Anthropic SSE response splits `message_start` (input=10,
  cache_read=5) and `message_delta` (output=25) across separate chunks
- **THEN** the settled record shows input=10, output=25, cache_read=5 and
  the client received the original bytes verbatim

#### Scenario: truncated stream tolerated

- **WHEN** the upstream connection drops mid-SSE after partial frames
- **THEN** the client receives the truncated bytes as-is and a usage record
  settles with whatever was parsed (all-null if nothing)

### Requirement: Buffered-body usage parsing

For non-SSE responses the router SHALL parse usage from the fully buffered
response JSON (top-level `usage` object) for both protocols and record it
with the response status.

#### Scenario: openai json usage

- **WHEN** an upstream returns a JSON body with
  `usage: {prompt_tokens: 12, completion_tokens: 34}`
- **THEN** the usage record shows input=12, output=34

### Requirement: Access log

Every request (including `/healthz`) SHALL emit one nginx-style access log
line via `tracing` at target `router::access`, on response-body completion
or client disconnect:

`{ip} - [{ts}] "{METHOD} {path}" {model}@{provider} {status} {bytes}
{latency}ms`

Model and provider are filled by the proxy from routing resolution;
requests that never reach routing (e.g. 401) show `-`. No key material is
logged. Output goes to stdout; there is no file writer or rotation.

#### Scenario: rejected request logged

- **WHEN** a request fails authentication with 401
- **THEN** the access log line shows status 401 with `-@-` for model and
  provider

#### Scenario: routed request logged

- **WHEN** a request for model `m1` routes to provider `alpha` and succeeds
  with 200 and 1024 bytes
- **THEN** the log line shows `"m1@alpha" 200 1024` and the latency in ms

### Requirement: Pipeline ordering

Middleware SHALL run in the order: access log (outermost) → authentication →
rate limiting → proxy handler, with `/healthz` whitelisted past both
authentication and rate limiting.

#### Scenario: auth before rate limit

- **WHEN** a request with an invalid key arrives while rate limiting is
  enabled
- **THEN** the response is 401 from authentication, and no rate-limit token
  is consumed
