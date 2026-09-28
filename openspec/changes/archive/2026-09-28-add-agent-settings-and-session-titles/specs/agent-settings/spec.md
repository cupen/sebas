## Purpose

Settings 里的 agent 目录管理：agent 定义住进 settings.db 并经 UI 全权增删改，config.toml 降级为种子源，agent 增删改免重启即时生效，builtIn（native，即 sebas agent）恒在。

## ADDED Requirements

### Requirement: Agent catalog lives in the state store

The settings database SHALL own the agent catalog: each user-manageable agent is a row in the `agents` table (id, driver tag, launch definition, display name, provenance, timestamps), and the built-in `native` kernel SHALL always appear in the catalog without being stored or deletable. Agent definitions SHALL be delivered to consumers as the `agents` snapshot domain over the core channel, and agent mutations SHALL commit through the same single-writer store discipline as the other settings.db tables.

#### Scenario: native kernel is always present

- **WHEN** the agent catalog is listed while the agents table is empty
- **THEN** the catalog contains exactly the `native` entry, marked built-in and non-deletable

#### Scenario: agents survive restart

- **WHEN** an agent is created through the UI and the processes restart
- **THEN** the agent is still listed with the same definition (persisted in the state store, not process memory)

### Requirement: Config agents seed the store

On startup the loader SHALL import `[acp.agents.<id>]` entries from config.toml into the agents store for ids that do not exist there yet (idempotent seeding). An id already present in the store SHALL NOT be overwritten by config — the store wins — and the loader SHALL log a startup notice naming the Settings surface for ignored entries. Config seeding SHALL NOT introduce a config writer: the UI SHALL NOT write back to config.toml.

#### Scenario: first startup imports config agents

- **WHEN** a fresh state directory starts with `[acp.agents.claude]` in config.toml
- **THEN** the agents store contains a `claude` row with the same launch definition, and `GET /api/agents` lists it

#### Scenario: store wins on the same id

- **WHEN** config.toml declares `[acp.agents.claude]` but the agents store already holds an edited `claude` row
- **THEN** startup keeps the store row unchanged and logs a notice that the config entry is ignored in favour of the Settings-managed one

#### Scenario: reseeding is idempotent

- **WHEN** the same config starts against a state directory it already seeded
- **THEN** the second startup creates no duplicate rows and changes nothing

### Requirement: Agents are managed from the Settings modal

The Settings modal SHALL expose an `Agents` section listing the built-in `native` entry (view-only) and every agents-store row with edit and delete actions. The create/edit form SHALL offer driver shapes `claude` (binary path, default `claude`), `opencode` (ACP command prefilled), and custom ACP (arbitrary command argv); stored definitions SHALL use the existing driver tags (`claude` | `acp`). Deleting an agent SHALL be a confirmed action and SHALL NOT affect already-created sessions.

#### Scenario: create an opencode agent without restart

- **WHEN** the operator adds an `opencode` agent via the form and saves
- **THEN** the new agent is selectable in the create-session dialog immediately, without restarting any process

#### Scenario: edit updates the catalog live

- **WHEN** the operator edits an agent's display name or launch command and saves
- **THEN** the catalog endpoint and the create-session dialog reflect the change without a restart

#### Scenario: delete removes the agent for new sessions only

- **WHEN** the operator deletes an agent that has an active session
- **THEN** the active session keeps working to the end of its lifecycle, and the agent disappears from the catalog and the create-session dialog

### Requirement: Spawn resolves agents dynamically

Session spawn SHALL resolve the requested agent id at spawn time from the union of the config-seeded registry and the agents store — an id absent from the config registry SHALL be resolved from the store and spawned with the stored definition. `sebas agent-kinds list` and the webui catalog SHALL report the union, each with honest reachability probing (command presence, version when available).

#### Scenario: store-only agent spawns

- **WHEN** a session is created with an agent id that exists only in the agents store
- **THEN** the session spawns with the stored definition and completes turns like any config-defined agent

#### Scenario: unknown agent is a typed rejection

- **WHEN** a session is created with an agent id present in neither the registry nor the store
- **THEN** the request is rejected with a typed unknown-agent error naming the id

### Requirement: Deleting an agent clears project defaults

When an agent is deleted and a project's remembered default agent refers to it, that project default SHALL be cleared (creation falls back to the catalog's normal default selection) instead of leaving a dangling reference.

#### Scenario: project default is cleared on delete

- **WHEN** the operator deletes agent `opencode` which is project P's remembered default agent
- **THEN** project P's default agent is cleared, and the create-session dialog for P preselects by the normal fallback rule
