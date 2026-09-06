## REMOVED Requirements

### Requirement: Configuration shape with backward-compatible migration

**Reason**: Nothing was released under the legacy `acp.claude` block, so the migration shim is pure noise. Configurations using `[acp.claude]` SHALL be rejected at parse time instead of silently rewritten.

**Migration**: Rewrite `[acp.claude] { … }` as `[acp.agents.claude] { driver = "claude", … }` and set `[acp] default = "claude"` if it was the only configured agent. The TOML parser reports the offending `[acp.claude]` line.

## ADDED Requirements

### Requirement: Legacy `[acp.claude]` block is rejected

Configurations using the legacy `[acp.claude]` table SHALL be rejected at parse time. The TOML deserializer reports the offending `[acp.claude]` line; the loader does not rewrite the block into `[acp.agents.claude]` and does not pick a default on the user's behalf. Configurations that declare only `[acp.agents.<kind>]` tables (with `default` either set explicitly or implicit when exactly one agent is configured) continue to load as today.

#### Scenario: Legacy `[acp.claude]` block fails parse

- **WHEN** the TOML config contains `[acp.claude]`
- **THEN** parsing fails with an error naming the `[acp.claude]` line and the loader does not produce a `Config`

#### Scenario: New `[acp.agents.claude]` block loads

- **WHEN** the TOML config contains
  `[acp.agents.claude] driver = "claude" path = "…" args = […]`
- **THEN** parsing succeeds and `cfg.acp.agents["claude"]` is set
- **AND** with no other agent configured and no `default` set, `cfg.acp.default`
  resolves to `"claude"` (implicit single-agent default)