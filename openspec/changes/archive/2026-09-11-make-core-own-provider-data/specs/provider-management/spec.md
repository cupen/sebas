## MODIFIED Requirements

### Requirement: Preset data follows the code table

The built-in preset table SHALL remain hardcoded in the application code and SHALL be
read-only data that is never rewritten by any surface. A provider that selects a preset
(by name or explicit `preset` field) SHALL resolve its base URLs from that table at
load/resolve time — never from a persisted copy — so an application update that changes a
preset's URLs automatically applies to every provider derived from it without any
stored-data change. An explicit base URL on a preset-derived provider SHALL be a
configuration error, not a silent overwrite of preset data.

The provider's **model list is core-owned data, not preset data**. It is seeded from the
preset's code-table list when the provider is created, and after that the operator may add,
remove, or reorder models through the ordinary provider-management surface. The code table
is therefore a starting point rather than the authority: the same preset can back providers
with different model lists, and a preset's own list is never modified by a provider edit.

#### Scenario: derived provider resolves from code

- **WHEN** a provider entry names preset `deepseek` with no URL or models fields of its own
- **THEN** it resolves with the `deepseek` preset's base URLs and models from the built-in
  table

#### Scenario: code preset update propagates

- **WHEN** the application updates preset `kimi`'s URLs and the user restarts (or
  hot-reloads) without touching stored provider data
- **THEN** the preset-derived provider `kimi` serves the new URLs

#### Scenario: URL override rejected

- **WHEN** a preset-derived provider entry sets any explicit base URL slot
- **THEN** configuration parsing fails with an error explaining that preset-derived
  providers follow the code table

#### Scenario: api key stays user-owned

- **WHEN** the user stores an API key on a preset-derived provider and the application later
  ships a changed preset
- **THEN** the stored key is untouched by the preset update

#### Scenario: model list is editable per provider

- **WHEN** the operator adds a model to a provider derived from preset `deepseek`
- **THEN** that provider's stored model list contains it while a second provider on the same
  preset keeps its own list

#### Scenario: editing a provider never rewrites the preset table

- **WHEN** the operator edits the model list of a provider derived from preset `deepseek`
- **THEN** the code table's `deepseek` entry is unchanged, and a provider created later from
  the same preset starts from the code-table list again

## ADDED Requirements

### Requirement: Core owns provider and model data

The provider and model store SHALL be owned by the core process: core SHALL be its only
writer, and the feishu `/provider` card, the WebUI provider surface, and any
operator-facing management path SHALL obtain and mutate provider, model, alias, and
default data through core. The router SHALL be a read-only consumer of that data — it
resolves and serves them, and SHALL NOT persist, create, rename, delete, or override them
by any path, including a file fallback. Default provider and default model SHALL be
stored with the provider data, not in a separate process-owned file.

#### Scenario: card edit is written by core

- **WHEN** the `/provider` card stores a provider
- **THEN** the write is committed by core, and a later read from the card, the WebUI, and the router all observe the same value

#### Scenario: router cannot write provider data

- **WHEN** a management operation is attempted against the router process while no core is reachable
- **THEN** it fails honestly with an unavailable-source error and no provider file is written or modified

#### Scenario: defaults live with provider data

- **WHEN** the operator sets the default provider and model
- **THEN** core stores them alongside provider data, and they survive both a router restart and a WebUI restart without a separate defaults file

#### Scenario: router reads a core change without a restart

- **WHEN** core commits a provider change while the router is running
- **THEN** the router picks it up through its subscription and routes by the new configuration without a restart
