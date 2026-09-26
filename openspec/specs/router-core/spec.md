# router-core Specification

## Purpose
Defines the router data plane: the dual-protocol endpoint surface and
protocol sniffing, model-to-provider routing (namespace, alias, default),
upstream forwarding with byte-faithful streaming, model renaming,
error translation, timeouts, and cancellation.

## Requirements

### Requirement: Endpoint surface

The router SHALL serve `GET /healthz` (literal `ok`), the admin surface
(`/admin/*` and `GET /metrics`, specified by the router-admin-api and
router-metrics capabilities — mounted above the catch-all proxy handler and
authenticated independently of proxy traffic), and handle every other path
through a single catch-all proxy handler. Paths under `/v1` or `/v1/*` are
proxied; paths with the explicit prefixes `/anthropic/*` and `/openai/*` are
proxied with the prefix stripped; any other non-admin path yields `404`
(`not_found`, OpenAI default shape).

#### Scenario: healthz

- **WHEN** a client requests `GET /healthz`
- **THEN** the response body is `ok` with no authentication

#### Scenario: non-v1 path rejected

- **WHEN** a client requests `POST /api/chat`
- **THEN** the response is 404 with a `not_found` error body

#### Scenario: admin route not swallowed by proxy fallback

- **WHEN** a client presents valid admin credentials to
  `GET /admin/stats`
- **THEN** the admin endpoint answers (not the proxy handler and not 404)

#### Scenario: retired provider routes answer 404

- **WHEN** a client requests any method on `/admin/providers` (the provider
  management surface moved behind the core state store)
- **THEN** the router answers 404 — the retired route is explicit, never
  swallowed into the proxy fallback

### Requirement: Bare-path protocol sniffing

For bare `/v1/*` paths the router SHALL determine the protocol from the
path and headers only (never the request body), in priority order: (1)
Anthropic path table — `/v1/messages` and its subpaths; (2) OpenAI
Responses path table — `/v1/responses` and its subpaths; (3) OpenAI
chat-completions path table — the remaining known OpenAI endpoints
(`/v1/chat/completions`, `/v1/embeddings`, and other OpenAI-specific
paths); (4) presence of an `anthropic-version` header forces Anthropic
(arbitrating collision paths such as `/v1/models`, `/v1/files`); (5)
default OpenAI chat-completions. Path matching is segment-boundary aware
(`/v1/messagesXYZ` does not match `/v1/messages`).

#### Scenario: messages path is anthropic

- **WHEN** a request hits `/v1/messages` with no protocol headers
- **THEN** it is routed as Anthropic protocol

#### Scenario: responses path is its own protocol

- **WHEN** a request hits `/v1/responses` (or a subpath of it) with no
  protocol headers
- **THEN** it is routed as the OpenAI Responses protocol, distinct from the
  chat-completions protocol

#### Scenario: collision arbitrated by header

- **WHEN** a request hits `/v1/models` with `anthropic-version: 2023-06-01`
- **THEN** it is routed as Anthropic; without the header it defaults to the
  OpenAI chat-completions protocol

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
(an unknown first segment falls through to normal matching); (2) model alias
exact match — an alias from the provider overlay file's `model_aliases`
routes to its bound provider (see router-model-aliases for translation
semantics); (3) the default provider. With exactly one provider configured
and no explicit default, that provider is the implicit default. The legacy
`[router.routes]` config table is retired: if present in the config it
SHALL be ignored with a deprecation warning and SHALL NOT contribute
routing entries.

#### Scenario: namespace routes directly

- **WHEN** the model is `openrouter/m1` and provider `openrouter` exists
- **THEN** the request routes to `openrouter` with upstream model `m1`

#### Scenario: unknown model without default

- **WHEN** the model matches no route and no default provider is configured
- **THEN** the response is 502 with error type `no_route`

#### Scenario: alias beats config route

- **WHEN** alias `m1` is bound to provider `beta` while a deprecated
  `[router.routes]` maps `m1` to provider `alpha`
- **THEN** a request for model `m1` routes to `beta` — the legacy table is
  ignored, so the alias stays authoritative

#### Scenario: exact beats glob

- **WHEN** a deprecated `[router.routes]` contains both `m*` and `m1` and
  the model is `m1`
- **THEN** the legacy table is not consulted at all — resolution proceeds
  by namespace, alias, and default provider in that order

### Requirement: Model rename

When the resolved provider's `model_map` maps the requested model to an
upstream name, the router SHALL rewrite the top-level `model` field of the
buffered JSON body to the mapped name (preserving all other fields) and use
the mapped name as the upstream model. Unmapped models pass through
unchanged with no body rewrite.

#### Scenario: rename applied

- **WHEN** model `m1` maps to `upstream-1` in the provider's `model_map`
- **THEN** the upstream request body carries `"model": "upstream-1"` and all
  other fields are preserved

### Requirement: Upstream request construction

The router SHALL construct the upstream request as: the provider's base
URL slot for the request protocol (`base_url_anthropic`,
`base_url_openai_chat`, or `base_url_openai_responses`) + the bare target
path + preserved query string. Request headers are sanitized — hop-by-hop
headers and the downstream credentials (`authorization`, `x-api-key`) are
stripped, business headers (`anthropic-version`, `anthropic-beta`,
`content-type`, custom `x-*`) pass through verbatim — and the provider's
API key injected: `x-api-key` for Anthropic upstreams, `Authorization:
Bearer` for both OpenAI-family upstreams (chat-completions and Responses).
The downstream key never appears in any forwarded header.

#### Scenario: anthropic key injection

- **WHEN** an Anthropic-protocol request is forwarded to an Anthropic
  upstream configured with key `up-key`
- **THEN** the upstream receives `x-api-key: up-key` and neither the
  downstream `authorization` nor downstream `x-api-key` header

#### Scenario: responses request uses its own slot

- **WHEN** a `/v1/responses` request routes to a provider whose
  `base_url_openai_responses` differs from `base_url_openai_chat`
- **THEN** the upstream URL is built from `base_url_openai_responses` and
  the upstream receives `Authorization: Bearer`

#### Scenario: query preserved

- **WHEN** the client requests `/v1/models?limit=1000`
- **THEN** the upstream URL ends with `/v1/models?limit=1000`

### Requirement: SSE byte passthrough

Responses with content-type `text/event-stream` SHALL be streamed to the
client chunk-by-chunk with bytes forwarded unmodified — no event parsing, no
re-framing, no injected heartbeat. The streaming decision is based solely on
the upstream response content-type, never on the request's `stream` flag.
Truncated or malformed upstream SSE frames pass through byte-for-byte
without causing a router error.

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

Router-generated errors SHALL be rendered in the protocol-appropriate shape
for the sniffed request protocol — Anthropic
`{"type":"error","error":{"type":...,"message":...}}` or OpenAI-family
(chat-completions and Responses alike)
`{"error":{"message":...,"type":...,"code":null}}` — with content-type
`application/json`. Status mapping: protocol mismatch (provider lacks the
base URL slot for the request protocol) → 400 `invalid_request_error`; no
route → 502 `no_route`; upstream unreachable, missing key, or missing URL →
502 `upstream_error`; non-`/v1` path → 404 `not_found`; body read failure →
400. Error messages SHALL be generic and never include downstream or
upstream keys. Upstream 4xx/5xx responses are NOT translated — they pass
through byte-for-byte with headers.

#### Scenario: dual-protocol error shape

- **WHEN** a no-route error occurs on an Anthropic-sniffed request
- **THEN** the body is `{"type":"error","error":{"type":"no_route",...}}`;
  the same failure on an OpenAI-family-sniffed request (chat-completions or
  Responses) renders the OpenAI `{"error":{...}}` shape

#### Scenario: upstream error passthrough

- **WHEN** the upstream returns 429 with a `retry-after` header
- **THEN** the client receives the same 429 body bytes and the preserved
  `retry-after` header

#### Scenario: no key leak

- **WHEN** any router-side error renders
- **THEN** the message text contains neither the downstream key nor the
  upstream key

### Requirement: Timeouts and cancellation

The router SHALL apply a connect timeout (default 10 s) and a per-read
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
- **THEN** the stream is not cut by the router

### Requirement: No protocol translation

The router SHALL forward same-protocol only: a resolved provider that lacks
the base URL slot for the request's protocol is a routing error
(`ProtocolMismatch` → 400), never a conversion between Anthropic and
OpenAI-family wire formats nor between the chat-completions and Responses
formats.

#### Scenario: protocol mismatch

- **WHEN** an Anthropic-protocol request routes to a provider with no
  `base_url_anthropic` slot
- **THEN** the response is 400 `invalid_request_error` naming the provider,
  and no upstream call is attempted

#### Scenario: responses request to chat-only provider

- **WHEN** a `/v1/responses` request routes to a provider that configures
  only `base_url_openai_chat`
- **THEN** the response is 400 `invalid_request_error` naming the provider;
  the request is never forwarded to the chat-completions slot

### Requirement: Debug test provider

With `--debug`, the router SHALL inject a built-in `test` provider and
`test → test` route: the router itself answers for all three protocols
(Anthropic, chat-completions, Responses) and both stream modes, skipping
upstream auth and key resolution and the protocol-consistency check. The
model name SHALL carry the scenario: the bare model `test` SHALL keep its
existing echo behavior exactly (fixed text echoing the last user message);
the namespaced forms `test/text`, `test/long`, `test/thinking`,
`test/tool-use`, `test/tools-parallel`, `test/full`, `test/empty`, and
`test/error` SHALL select deterministic response scenarios, each mapped to
core capabilities they exercise. Scenario responses SHALL be composed of
Anthropic content blocks of the kinds the scenario names, and identical
requests SHALL produce identical responses.

For `test/tool-use` and `test/full`, the provider SHALL follow
deterministic agent-loop rules identical in semantics to the fake
upstream's: a request carrying a non-empty `tools` array whose history has
no `tool_result` SHALL be answered with a `tool_use` block naming the
first tool (stop reason `tool_use`, deterministic input); a history that
already contains a `tool_result` SHALL be answered with final text (stop
reason `end_turn`); a request without tools SHALL be answered with plain
text. `test/tools-parallel` SHALL instead answer the first such turn with
a `tool_use` block per declared tool — deterministic distinct inputs — so
parallel tool calls, each with its own permission request, can be driven;
subsequent turns follow the final-text rule. The text-family scenarios
(`text`, `long`, `thinking`, `empty`) SHALL NOT emit tool-use blocks.

`test/long` SHALL produce a long deterministic text body streamed in many
fixed-size chunks. `test/empty` SHALL complete the turn with zero content
blocks. `test/error` SHALL answer with an Anthropic error body (HTTP 5xx,
`api_error`) instead of a message. Streaming responses SHALL use the event
sequence appropriate to each block kind — thinking deltas, input-json
deltas, text deltas — with deterministic chunking, and the final stop
reason SHALL match the scenario. Each message-producing scenario SHALL
report a fixed non-zero token usage (`test` and `test/empty` keep their
existing all-zero usage). On the OpenAI protocol family, the thinking
scenario SHALL degrade to plain text, and `test/error` SHALL produce the
family's error shape.

#### Scenario: debug echo

- **WHEN** the router runs with `--debug` and a request carries model
  `test`
- **THEN** the response is served locally, echoing the request's user
  content, with no upstream contact
- **AND** the response is byte-identical to the pre-scenario behavior

#### Scenario: thinking scenario emits thinking then text

- **WHEN** a request carries model `test/thinking` (non-streaming)
- **THEN** the response content contains a thinking block followed by a
  text block
- **AND** streaming the same request emits thinking deltas before text
  deltas within the correct block events

#### Scenario: tool-use scenario drives the agent loop

- **WHEN** a request carries model `test/tool-use` with a non-empty
  `tools` array and no `tool_result` in history
- **THEN** the response contains a `tool_use` block naming the first
  tool with stop reason `tool_use`
- **AND** when the history already contains a `tool_result`, the response
  is final text with stop reason `end_turn`

#### Scenario: parallel tools scenario emits one tool_use per tool

- **WHEN** a request carries model `test/tools-parallel` with two or more
  declared tools and no `tool_result` in history
- **THEN** the response contains one `tool_use` block per declared tool,
  each with a deterministic distinct input
- **AND** the turn ends with stop reason `tool_use`

#### Scenario: long scenario streams deterministic long text

- **WHEN** a request carries model `test/long` with streaming enabled
- **THEN** the response streams a long deterministic text body in many
  fixed-size text deltas
- **AND** the assembled text is identical to the non-streaming body

#### Scenario: empty scenario completes without content

- **WHEN** a request carries model `test/empty`
- **THEN** the turn completes successfully with zero content blocks and
  stop reason `end_turn`

#### Scenario: error scenario answers with an upstream-style error

- **WHEN** a request carries model `test/error`
- **THEN** the response is an Anthropic error body with HTTP 5xx and type
  `api_error`
- **AND** no message is produced

#### Scenario: scenarios are deterministic

- **WHEN** the same scenario request is sent twice
- **THEN** both responses have identical content blocks, stop reason, and
  usage

#### Scenario: thinking degrades on the OpenAI family

- **WHEN** a `test/thinking` request arrives on the chat-completions
  protocol
- **THEN** the response is plain text
- **AND** no protocol error is produced

### Requirement: Three-slot provider base URLs

A provider configuration SHALL carry three independent base URL slots:
`base_url_anthropic` (Anthropic wire protocol), `base_url_openai_chat`
(OpenAI chat-completions wire protocol), and `base_url_openai_responses`
(OpenAI Responses wire protocol). The unified `base_url_openai` slot is
removed — nothing was released under the old name, so no alias, fallback,
or migration is kept. A provider serves only the protocols whose slots are
configured; a custom provider MUST configure at least one slot, and a
request for a protocol whose slot is absent is a routing error, never a
fallback to a sibling slot.

#### Scenario: three slots resolve independently

- **WHEN** a provider defines `base_url_anthropic` and
  `base_url_openai_responses` but not `base_url_openai_chat`
- **THEN** Anthropic and Responses requests resolve for that provider while
  a chat-completions request is a protocol mismatch

#### Scenario: custom provider needs at least one slot

- **WHEN** a custom provider (no preset) is declared with none of the three
  slots
- **THEN** configuration parsing fails with an error naming the provider and
  the three slot names

#### Scenario: old unified slot is not read

- **WHEN** a provider entry carries only the removed `base_url_openai` field
- **THEN** the field is rejected as unknown rather than silently serving any
  protocol
