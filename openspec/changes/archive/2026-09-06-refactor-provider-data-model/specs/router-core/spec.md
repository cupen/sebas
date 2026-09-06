## ADDED Requirements

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

## MODIFIED Requirements

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
`test → test` route: the router itself answers (echoing the last user
message) for all three protocols (Anthropic, chat-completions, Responses)
and both stream modes, skipping upstream auth and key resolution and the
protocol-consistency check.

#### Scenario: debug echo

- **WHEN** the router runs with `--debug` and a request carries model
  `test`
- **THEN** the response is served locally, echoing the request's user
  content, with no upstream contact
