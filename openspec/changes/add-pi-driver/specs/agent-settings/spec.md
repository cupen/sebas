## MODIFIED Requirements

### Requirement: Agents are managed from the Settings modal

The Settings modal SHALL expose an `Agents` section listing the built-in `native` entry (view-only) and every agents-store row with edit and delete actions. The create/edit form SHALL offer driver shapes `claude` (binary path, default `claude`), `pi` (binary path default `pi`，另含 sessions 目录字段，镜像 claude 形态), `opencode` (ACP command prefilled), and custom ACP (arbitrary command argv); stored definitions SHALL use the existing driver tags (`claude` | `acp` | `pi`). Deleting an agent SHALL be a confirmed action and SHALL NOT affect already-created sessions.

#### Scenario: create an opencode agent without restart

- **WHEN** the operator adds an `opencode` agent via the form and saves
- **THEN** the new agent is selectable in the create-session dialog immediately, without restarting any process

#### Scenario: create a pi agent from the form

- **WHEN** the operator adds a `pi` agent via the form（driver 形态选 pi，填二进制路径与 sessions 目录）并保存
- **THEN** the stored row carries driver tag `pi`, and a session created with it spawns via the Pi driver without restart

#### Scenario: edit updates the catalog live

- **WHEN** the operator edits an agent's display name or launch command and saves
- **THEN** the catalog endpoint and the create-session dialog reflect the change without a restart

#### Scenario: delete removes the agent for new sessions only

- **WHEN** the operator deletes an agent that has an active session
- **THEN** the active session keeps working to the end of its lifecycle, and the agent disappears from the catalog and the create-session dialog
