# router-model-aliases Specification

## Purpose
模型别名：给模型起自定义名称并绑定到指定 provider 的路由实体——客户端用别名发请求，router 将其路由到绑定的 provider 并按需改写为上游真实模型名。

## Requirements

### Requirement: Alias entity and persistence

A model alias SHALL be persisted in the core state store as a row with fields: `alias` (the custom name clients use), `provider` (the bound provider name), and optional `upstream_model` (the real model name forwarded upstream). Aliases survive restarts, are read and written through the core channel state methods (in-process within core), and coexist with other stored data without affecting it.

#### Scenario: alias persists across restart

- **WHEN** an alias `my-claude` is created and the router process restarts
- **THEN** requests for model `my-claude` still route to the bound provider

### Requirement: Alias resolution precedence

Alias matching SHALL sit in the routing priority chain between the provider
namespace and exact config routes: (1) provider namespace (`provider/model`,
first segment names a known provider); (2) alias exact match; (3) exact
config route; (4) glob route; (5) default provider. An alias whose name
equals a config-seed route name MUST win over the config route. Alias
matching is exact — aliases never participate in glob matching.

#### Scenario: alias routes to bound provider

- **WHEN** alias `my-claude` is bound to provider `company` and a request
  names model `my-claude`
- **THEN** the request routes to `company`

#### Scenario: alias beats same-named config route

- **WHEN** alias `m1` is bound to provider `beta` while a config route `m1`
  points to provider `alpha`
- **THEN** a request for model `m1` routes to `beta`

#### Scenario: namespace still wins over alias

- **WHEN** a provider named `beta` exists and the request model is
  `beta/m1`
- **THEN** the namespace path routes to provider `beta` with upstream model
  `m1`, regardless of any alias named `beta/m1` or `m1`

### Requirement: Upstream model translation

When an alias defines `upstream_model`, the router SHALL rewrite the
request's top-level `model` field to that value before forwarding (same
best-effort semantics as model-map renaming: non-JSON or field-less bodies
forward unchanged). When `upstream_model` is omitted, the request's model
string MUST be forwarded unchanged, with the request still pinned to the
bound provider.

#### Scenario: alias with upstream model renames

- **WHEN** alias `my-claude` has `upstream_model: claude-sonnet-4` and a
  request body carries `model: my-claude`
- **THEN** the forwarded body carries `model: claude-sonnet-4`

#### Scenario: alias without upstream model passes through

- **WHEN** alias `fast` is bound to provider `beta` without an
  `upstream_model` and a request names model `fast`
- **THEN** the forwarded request keeps model `fast` and goes to `beta`

### Requirement: Alias validation

Alias writes MUST be validated before persistence: the alias is non-empty,
does not contain `/` (reserved for the namespace syntax), and references an
existing provider. Aliases introduced by external edits that fail validation
are dropped at load time with a warning; the remaining aliases still apply
(partial self-heal, never a startup failure).

#### Scenario: externally broken alias dropped with warning

- **WHEN** the provider overlay file is hand-edited to contain an alias
  referencing a non-existent provider
- **THEN** the router starts or reloads with that alias dropped, logs a
  warning naming the alias, and other aliases remain effective

### Requirement: Alias scope

Model aliases SHALL affect router routing only. Direct and Off provider
modes translate spawn-time environment independently and MUST NOT consult or
rewrite aliases.

#### Scenario: direct mode ignores aliases

- **WHEN** Direct mode spawns an agent with a model string equal to an alias
  name
- **THEN** the spawn translation passes the model through unchanged without
  consulting the alias table
