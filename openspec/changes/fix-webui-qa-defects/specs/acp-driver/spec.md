## ADDED Requirements

### Requirement: Agent argv fidelity

Configured `args` for a claude-driver agent SHALL reach the spawned child's
argv without loss. The internal flag-map representation cannot express
positional arguments; therefore the config parser SHALL reject any positional
argument at parse time with an actionable error naming the argument and the
keyed (`--flag value`) alternative, instead of dropping it with only a runtime
log warning. A configuration that passes parsing SHALL produce a child argv
containing exactly the configured flags and values.

#### Scenario: positional arg is rejected at parse time

- **WHEN** a config declares `args = ["thinking"]` (a bare positional) for a claude-driver agent
- **THEN** configuration parsing fails with an error naming the offending argument and suggesting the keyed form (for example `--scenario thinking`)

#### Scenario: keyed args reach the child argv

- **WHEN** a config declares `args = ["--scenario", "thinking"]`
- **THEN** the spawned child's argv contains `--scenario thinking` and the child observes the intended value

#### Scenario: silent drop no longer happens

- **WHEN** an agent session spawns with configured args
- **THEN** no configured argument is silently omitted from the child argv, and no "dropping positional claude arg" warning exists at runtime because such configs are rejected at parse time
