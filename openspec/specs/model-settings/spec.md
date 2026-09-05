## Purpose

Give the operator a WebUI-native management surface for model providers —
the equivalent of deepseek-harness's Settings → Models: providers configured
where they are used, credential state visible at a glance, and the agent's
model catalog derived from what is actually configured rather than from
environment variables.

## Requirements

### Requirement: Provider management API in the WebUI

The WebUI SHALL expose provider management endpoints (list, create, update,
delete) over its HTTP API. List responses SHALL include each provider's name,
preset, base URLs, protocol, models catalog, default model, and a masked
indication of whether an API key is configured — never the key itself.
Mutations SHALL write through the state store — the same store and semantics
as the Feishu `/provider` card — so both surfaces stay consistent without a
sync step. Deletion SHALL follow the existing soft-delete and
default-clearing semantics.

#### Scenario: list masks secrets

- **WHEN** the client requests the provider list and provider `alpha` has an
  API key configured
- **THEN** the response marks `alpha` as key-configured and contains no
  substring of the actual key

#### Scenario: mutation reaches both surfaces

- **WHEN** the operator renames provider `alpha` in the WebUI settings, then
  opens the Feishu `/provider` card
- **THEN** the card shows the same name without any manual refresh or sync

#### Scenario: delete clears default

- **WHEN** the operator deletes the provider that is currently the agent
  default
- **THEN** the default selection is cleared rather than left dangling

### Requirement: Model catalog probing

The WebUI SHALL offer a probe action per provider that fetches the provider's
model catalog from its `/models` endpoint and writes the result back to that
provider's catalog, reusing the same endpoint derivation and error semantics
as the existing Feishu-side probe. Probing SHALL be executable from both
deployment forms.

#### Scenario: probe refreshes the catalog

- **WHEN** the operator triggers probe on a provider with a reachable
  `/models` endpoint
- **THEN** the provider's catalog is replaced with the fetched model list and
  the response reports success

#### Scenario: probe failure is reported

- **WHEN** the probe cannot reach the endpoint
- **THEN** the response reports the failure with its cause and the existing
  catalog is left unchanged

### Requirement: Agent model defaults follow provider config

The operator SHALL be able to select the agent's default provider and default
model in the WebUI. When a default provider with a non-empty models catalog is
configured, the native agent backend's available-model list and initial model
SHALL derive from that provider's catalog and default model. The
`SEBAS_AGENT_MODELS` / `SEBAS_AGENT_MODEL` environment variables SHALL remain
supported as explicit overrides that take precedence over the derived values.
With no default provider configured, the backend SHALL report honest
unavailability for model selection rather than a fabricated catalog.

#### Scenario: native dropdown reflects configured provider

- **WHEN** provider `alpha` is set as default with catalog
  `[a1, a2, a3]` and default model `a2`, and the operator opens the model
  selector for a native session
- **THEN** the selector offers `a1`, `a2`, `a3` with `a2` current

#### Scenario: env override still wins

- **WHEN** `SEBAS_AGENT_MODELS` is set to `x1,x2` while provider config also
  exists
- **THEN** the native backend offers exactly `x1`, `x2`

#### Scenario: no provider configured is honest

- **WHEN** no default provider is configured and no env override is set
- **THEN** the native backend reports model selection as unavailable with that
  cause, not an empty or fabricated list

### Requirement: Models settings management UI

The WebUI settings' Models section SHALL be a management surface, not a
read-only list: one card per provider summarizing name, configured state
(key configured or not), base URL, model count, and the default mark; create
and edit forms (preset and custom variants) capturing the same fields as the
Feishu card; per-provider probe and delete actions; and the default
provider/model selection. Credential inputs SHALL be secret-masked. The
section SHALL render in both deployment forms against the same API.

#### Scenario: provider cards summarize state

- **WHEN** providers `alpha` (key configured, 12 models) and `beta` (no key)
  exist and `alpha` is default
- **THEN** the Models section shows a card for each with alpha's card marked
  default and beta's card marked not-configured

#### Scenario: create via preset form

- **WHEN** the operator adds a provider through the preset form providing
  name, preset, and API key
- **THEN** the provider is created with the preset-derived base URLs and
  appears in the card list

#### Scenario: default selection persists

- **WHEN** the operator selects `alpha` and model `a2` as agent defaults and
  reloads the page
- **THEN** the selection still shows `alpha` / `a2`
