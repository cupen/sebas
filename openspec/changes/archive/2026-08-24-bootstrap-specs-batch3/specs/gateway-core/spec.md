## Purpose

Defines the gateway data plane: the dual-protocol endpoint surface and
protocol sniffing, model-to-provider routing (exact, glob, namespace,
default), upstream forwarding with byte-faithful streaming, model renaming,
error translation, timeouts, and cancellation.

## ADDED Requirements

### Requirement: Endpoint surface

The gateway SHALL serve `GET /healthz` (literal `ok`) and handle every other
path through a single catch-all proxy handler. Paths under `/v1` or `/v1/*`
are proxied; paths with the explicit prefixes `/anthropic/*` and `/openai/*`
are proxied with the prefix stripped; any other path yields `404`
(`not_found`, OpenAI default shape).

#### Scenario: healthz

- **WHEN** a client requests `GET /healthz`
- **THEN** the response body is `ok` with no authentication

#### Scenario: non-v1 path rejected

- **WHEN** a client requests `POST /api/chat`
- **THEN** the response is 404 with a `not_found` error body

### Requirement: Bare-path protocol sniffing

For bare `/v1/*` paths the gateway SHALL determine the protocol from the
path and headers only (never the request body), in priority order: (1)
Anthropic path table — `/v1/messages` and its subpaths; (2) OpenAI path
table — the known OpenAI endpoints (`/v1/chat/completions`,
`/v1/responses`, `/v1/embeddings`, and other OpenAI-specific paths);
(3) presence of an `anthropic-version` header forces Anthropic (arbitrating
collision paths such as `/v1/models`, `/v1/files`); (4) default OpenAI.
Path matching is segment-boundary aware (`/v1/messagesXYZ` does not match
`/v1/messages`).

#### Scenario: messages path is anthropic

- **WHEN** a request hits `/v1/messages` with no protocol headers
- **THEN** it is routed as Anthropic protocol

#### Scenario: collision arbitrated by header

- **WHEN** a request hits `/v1/models` with `anthropic-version: 2023-06-01`
- **THEN** it is routed as Anthropic; without the header it defaults to
  OpenAI

#### Scenario: segment boundary

- **WHEN** a request hits `/v1/messagesXYZ`
- **THEN** the path matches neither protocol table entry and falls through
  to header/default resolution

### Requirement: Explicit prefix mounting

Requests under `/anthropic/*` SHALL be forced to Anthropic protocol and
`/openai/*` to OpenAI protocol, with the prefix stripped before forwarding —
the forced protocol wins even when the remaining path would otherwise sniff
as the opposite protocol. Prefix matching is segment-boundary aware.

#### Scenario: prefix forces protocol

- **WHEN** a request hits `/anthropic/v1/chat/completions`
- **THEN** it is forwarded to the upstream as Anthropic protocol at bare
  path `/v1/chat/completions`

### Requirement: Model extraction

The model SHALL be extracted from the top-level `model` field of the
request JSON body when the request is a buffer-method (POST/PUT/PATCH) with
a JSON content-type, and otherwise from the path for `/v1/models/{id}`
single-segment ids. GET/DELETE and non-JSON bodies are never buffered —
their model, if any, comes only from the path.

#### Scenario: model from body

- **WHEN** a POST to `/v1/messages` has body `{"model": "m1", ...}`
- **THEN** routing resolves against model `m1`

#### Scenario: model from path

- **WHEN** a GET hits `/v1/models/m1`
- **THEN** routing resolves against model `m1`

### Requirement: Body buffering and replay

Buffered request bodies SHALL be capped at `max_body_bytes` (default 64
MiB) — exceeding the limit yields `413`; a body read error yields `400`. The
buffered body is replayed upstream verbatim (after optional model renaming)
rather than streamed from the client.

#### Scenario: oversized body

- **WHEN** a POST body exceeds `max_body_bytes`
- **THEN** the response is 413 without contacting any upstream

### Requirement: Routing resolution order

The routing table SHALL resolve a model in priority order: (1) provider
namespace — a `provider/model` string whose first segment names a known
provider routes to that provider with the remainder as the upstream model
(an unknown first segment falls through to normal matching); (2) exact model
match; (3) glob match (`*` patterns, deterministic under collision —
lexicographically first); (4) the default provider. A route group listing
multiple providers uses only the first. With exactly one provider configured
and no explicit default, that provider is the implicit default.

#### Scenario: namespace routes directly

- **WHEN** the model is `openrouter/m1` and provider `openrouter` exists
- **THEN** the request routes to `openrouter` with upstream model `m1`

#### Scenario: exact beats glob

- **WHEN** routes `m*` and `m1` both exist and the model is `m1`
- **THEN** the exact route wins regardless of declaration order

#### Scenario: unknown model without default

- **WHEN** the model matches no route and no default provider is configured
- **THEN** the response is 502 with error type `no_route`

### Requirement: Model rename

When the resolved provider's `model_map` maps the requested model to an
upstream name, the gateway SHALL rewrite the top-level `model` field of the
buffered JSON body to the mapped name (preserving all other fields) and use
the mapped name as the upstream model. Unmapped models pass through
unchanged with no body rewrite.

#### Scenario: rename applied

- **WHEN** model `m1` maps to `upstream-1` in the provider's `model_map`
- **THEN** the upstream request body carries `"model": "upstream-1"` and all
  other fields are preserved

### Requirement: Upstream request construction

The gateway SHALL construct the upstream request as: provider `base_url`
for the request protocol + the bare target path + preserved query string.
Request headers are sanitized — hop-by-hop headers and the downstream
credentials (`authorization`, `x-api-key`) are stripped, business headers
(`anthropic-version`, `anthropic-beta`, `content-type`, custom `x-*`) pass
through verbatim — and the provider's API key injected: `x-api-key` for
Anthropic upstreams, `Authorization: Bearer` for OpenAI upstreams. The
downstream key never appears in any forwarded header.

#### Scenario: anthropic key injection

- **WHEN** an Anthropic-protocol request is forwarded to an Anthropic
  upstream configured with key `up-key`
- **THEN** the upstream receives `x-api-key: up-key` and neither the
  downstream `authorization` nor downstream `x-api-key` header

#### Scenario: query preserved

- **WHEN** the client requests `/v1/models?limit=1000`
- **THEN** the upstream URL ends with `/v1/models?limit=1000`

### Requirement: SSE byte passthrough

Responses with content-type `text/event-stream` SHALL be streamed to the
client chunk-by-chunk with bytes forwarded unmodified — no event parsing, no
re-framing, no injected heartbeat. The streaming decision is based solely on
the upstream response content-type, never on the request's `stream` flag.
Truncated or malformed upstream SSE frames pass through byte-for-byte
without causing a gateway error.

#### Scenario: byte-for-byte sse

- **WHEN** an upstream streams an Anthropic SSE response
- **THEN** the client receives exactly the bytes the upstream sent,
  including any `ping` frames, and the `text/event-stream` content-type is
  preserved

#### Scenario: truncated stream

- **WHEN** the upstream connection drops mid-SSE
- **THEN** the already-received bytes are relayed as-is and no 502 is
  synthesized for the truncation

### Requirement: Buffered non-SSE relay

Non-SSE responses SHALL be fully buffered from the upstream and relayed
with upstream status and sanitized headers (hop-by-hop and content-length
stripped; business headers like `retry-after` and `x-request-id`
preserved). An upstream body-read failure yields `502 upstream_error`.

#### Scenario: json relay

- **WHEN** the upstream returns a JSON completion
- **THEN** the client receives the identical body bytes with the upstream
  status code

### Requirement: Error translation

Gateway-generated errors SHALL be rendered in the protocol-appropriate shape
for the sniffed request protocol — Anthropic
`{"type":"error","error":{"type":...,"message":...}}` or OpenAI
`{"error":{"message":...,"type":...,"code":null}}` — with content-type
`application/json`. Status mapping: protocol mismatch (provider lacks a URL
for the request protocol) → 400 `invalid_request_error`; no route → 502
`no_route`; upstream unreachable, missing key, or missing URL → 502
`upstream_error`; non-`/v1` path → 404 `not_found`; body read failure → 400.
Error messages SHALL be generic and never include downstream or upstream
keys. Upstream 4xx/5xx responses are NOT translated — they pass through
byte-for-byte with headers.

#### Scenario: dual-protocol error shape

- **WHEN** a no-route error occurs on an Anthropic-sniffed request
- **THEN** the body is `{"type":"error","error":{"type":"no_route",...}}`;
  the same failure on an OpenAI-sniffed request renders the OpenAI
  `{"error":{...}}` shape

#### Scenario: upstream error passthrough

- **WHEN** the upstream returns 429 with a `retry-after` header
- **THEN** the client receives the same 429 body bytes and the preserved
  `retry-after` header

#### Scenario: no key leak

- **WHEN** any gateway-side error renders
- **THEN** the message text contains neither the downstream key nor the
  upstream key

### Requirement: Timeouts and cancellation

The gateway SHALL apply a connect timeout (default 10 s) and a per-read
timeout (default 600 s) that resets on upstream activity — a live SSE stream
that keeps emitting is never cut. There is no hard total-request timeout.
Client disconnect SHALL implicitly cancel the upstream request through
response-body drop.

#### Scenario: idle read timeout

- **WHEN** the upstream stalls with no bytes for the read timeout
- **THEN** the response is 502 `upstream_error`

#### Scenario: long-lived stream survives

- **WHEN** an SSE stream emits chunks steadily for longer than the read
  timeout
- **THEN** the stream is not cut by the gateway

### Requirement: No protocol translation

The gateway SHALL forward same-protocol only: a resolved provider that lacks
a base URL for the request's protocol is a routing error (`ProtocolMismatch`
→ 400), never a conversion between Anthropic and OpenAI wire formats.

#### Scenario: protocol mismatch

- **WHEN** an Anthropic-protocol request routes to a provider with only an
  OpenAI base URL
- **THEN** the response is 400 `invalid_request_error` naming the provider,
  and no upstream call is attempted

### Requirement: Debug test provider

With `--debug`, the gateway SHALL inject a built-in `test` provider and
`test → test` route: the gateway itself answers (echoing the last user
message) for both protocols and both stream modes, skipping upstream auth
and key resolution and the protocol-consistency check.

#### Scenario: debug echo

- **WHEN** the gateway runs with `--debug` and a request carries model
  `test`
- **THEN** the response is served locally, echoing the request's user
  content, with no upstream contact
