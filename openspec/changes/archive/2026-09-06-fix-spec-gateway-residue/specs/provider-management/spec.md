## MODIFIED Requirements

### Requirement: /provider main card layout

The `/provider` command SHALL render a single management card with four
sections, top to bottom: (1) three mode buttons `Off` / `Direct` / `Router`
with the current mode rendered `primary`; (2) a DIRECT-mode default provider
dropdown (options: all provider names in alphabetical order plus
`（未设置）`); (3) the provider list — one collapsed `collapsible_panel` per
provider whose header summarizes name, DIRECT-default mark, and default
model, and whose body shows markdown field rows (preset, base URLs, API key
configured-or-not, default model) plus four buttons (probe models, edit,
delete, set as DIRECT default); (4) a create sub-section with
`＋ 新增（预设）` and `＋ 新增（自定义）` buttons.

#### Scenario: card sections

- **WHEN** the user sends `/provider`
- **THEN** the bot sends one card containing the mode buttons, the default
  dropdown, the collapsed provider panels, and the two create buttons

#### Scenario: default dropdown options

- **WHEN** providers `zeta` and `alpha` exist and no default is selected
- **THEN** the dropdown offers `（未设置）`, `alpha`, `zeta` in that order

### Requirement: Mode switching

Clicking a mode button SHALL write the mode to the state file and refresh the
management card in place. Switching to `Direct` while no default provider is
selected SHALL auto-fill the alphabetically-first provider as the default
(without setting a default model). The persisted mode value SHALL be
`router`; a state file carrying the pre-rename value `gateway` SHALL fail
to parse — nothing was released under the old value, so no alias is kept.

#### Scenario: direct auto-fill

- **WHEN** the user clicks `Direct` with providers `alpha` and `zeta`
  configured and no default selection
- **THEN** the state file records mode `direct` with default provider
  `alpha`, and the refreshed card marks `alpha` as the DIRECT default

#### Scenario: router mode write

- **WHEN** the user clicks `Router`
- **THEN** the state file records mode `router` and the card refreshes with
  `Router` rendered as the active mode

#### Scenario: pre-rename state value rejected

- **WHEN** the state file contains `"kind": "gateway"`
- **THEN** parsing fails with an error naming the state file instead of
  silently guessing the mode

### Requirement: Model flag precedence

When constructing the `--model` arg, the spawn SHALL apply precedence:
`default_selection.model` (only when the default selection's provider is the
resolved provider) over the provider's `default_model`, over no flag. In
Router mode the `--model` flag SHALL never be added.

#### Scenario: default selection wins

- **WHEN** `default_selection` is provider `alpha` model `m1` and `alpha`'s
  `default_model` is `m2`
- **THEN** the child receives `--model m1`

#### Scenario: overlay default model fallback

- **WHEN** `default_selection` names provider `alpha` with no model and
  `alpha`'s `default_model` is `m2`
- **THEN** the child receives `--model m2`

#### Scenario: router never pins model

- **WHEN** mode is `router` and both default selection model and provider
  default model exist
- **THEN** no `--model` arg is passed

### Requirement: Router mode env translation

In `Router` mode the spawn SHALL translate the router's `listen` address
and first `auth_token` into `ANTHROPIC_BASE_URL=http://{listen}` +
`ANTHROPIC_AUTH_TOKEN={token}` — the router always presents the Anthropic
protocol face to the agent. An empty `listen` is a resolution error; an
empty `auth_token` proceeds with a warning.

#### Scenario: router env construction

- **WHEN** mode is `router` with `listen = "127.0.0.1:8787"` and
  `auth_token = ["sk-x"]`
- **THEN** the child env sets `ANTHROPIC_BASE_URL=http://127.0.0.1:8787` and
  `ANTHROPIC_AUTH_TOKEN=sk-x`

#### Scenario: missing listen aborts

- **WHEN** mode is `router` and the router config has an empty `listen`
- **THEN** the spawn resolves to an error and the child is not started with
  partial router env
