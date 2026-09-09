## MODIFIED Requirements

### Requirement: Provider-driven environment injection

The system SHALL merge `extra_env` (e.g. `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `OPENAI_BASE_URL`, `OPENAI_API_KEY`, and the model cover set from `claude-env-cover`) into the child process environment at spawn. Entries the resolution marks as cover variables SHALL override any OS-inherited value; all other entries SHALL merge on top of the OS environment. The same injection SHALL apply to both fresh spawns and resumes.

#### Scenario: Direct mode injects Anthropic env

- **WHEN** the router resolves provider mode to Direct with an Anthropic-protocol provider
- **THEN** the spawn passes `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` via `extra_env`
- **AND** the child uses those values instead of any OS-level values

#### Scenario: Cover variables override inherited values

- **WHEN** the OS environment carries `ANTHROPIC_MODEL=some-other-model` and the resolved cover set derives the provider's strongest model
- **THEN** the child sees the derived value, not the inherited one

#### Scenario: Resume applies the same injection

- **WHEN** a Claude child is resumed with the same provider resolution as its original spawn
- **THEN** the child env carries the same `extra_env`, including the cover variables
