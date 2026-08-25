## MODIFIED Requirements

### Requirement: Endpoint surface

The gateway SHALL serve `GET /healthz` (literal `ok`), the admin surface
(`/admin/*` and `GET /metrics`, specified by the gateway-admin-api and
gateway-metrics capabilities — mounted above the catch-all proxy handler and
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
  `GET /admin/providers`
- **THEN** the admin endpoint answers (not the proxy handler and not 404)

### Requirement: Routing resolution order

The routing table SHALL resolve a model in priority order: (1) provider
namespace — a `provider/model` string whose first segment names a known
provider routes to that provider with the remainder as the upstream model
(an unknown first segment falls through to normal matching); (2) model alias
exact match — an alias from the provider overlay file's `model_aliases`
routes to its
bound provider (see gateway-model-aliases for translation semantics) and
wins over a same-named config route; (3) exact model match; (4) glob match
(`*` patterns, deterministic under collision — lexicographically first);
(5) the default provider. A route group listing multiple providers uses only
the first. With exactly one provider configured and no explicit default,
that provider is the implicit default.

#### Scenario: namespace routes directly

- **WHEN** the model is `openrouter/m1` and provider `openrouter` exists
- **THEN** the request routes to `openrouter` with upstream model `m1`

#### Scenario: alias beats config route

- **WHEN** alias `m1` is bound to provider `beta` while a config route `m1`
  points to provider `alpha`
- **THEN** a request for model `m1` routes to `beta`

#### Scenario: exact beats glob

- **WHEN** routes `m*` and `m1` both exist and the model is `m1`
- **THEN** the exact route wins regardless of declaration order

#### Scenario: unknown model without default

- **WHEN** the model matches no route and no default provider is configured
- **THEN** the response is 502 with error type `no_route`
