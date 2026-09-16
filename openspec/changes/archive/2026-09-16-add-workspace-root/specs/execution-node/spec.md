## ADDED Requirements

### Requirement: The node enforces its own workspace root

The execution node SHALL resolve its own workspace root with the same posture as the control plane — `SEBAS_WORKSPACE_ROOT` environment variable > node config entry > fallback to the node process working directory with a startup warning — independently of the control plane's value. When the control plane asks the node to judge a project path, the node SHALL apply its workspace-root containment in addition to existence and directory checks: a path that resolves outside the node's workspace root, or cannot be resolved, SHALL be judged out of scope and rejected as unusable for spawning.

#### Scenario: node rejects an out-of-root registration

- **WHEN** the control plane asks the node to judge a project path that resolves outside the node's workspace root
- **THEN** the node reports the path as out of scope, and the control plane does not register the project against that node

#### Scenario: node accepts an in-root path

- **WHEN** the control plane asks the node to judge a project path that resolves inside the node's workspace root
- **THEN** the node reports the path as in scope alongside its existence and directory facts

#### Scenario: node falls back to its working directory with a warning

- **WHEN** the node starts with neither the environment variable nor a config entry for the workspace root
- **THEN** the node uses its process working directory as the root and logs a startup warning recommending explicit configuration
