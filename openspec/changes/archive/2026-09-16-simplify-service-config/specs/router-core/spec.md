## MODIFIED Requirements

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
