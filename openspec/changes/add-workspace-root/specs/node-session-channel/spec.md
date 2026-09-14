## MODIFIED Requirements

### Requirement: Project references cross the link as node and path

A session's project SHALL cross the link as a reference to a directory path on the node. The judgement of whether that path is usable SHALL be made by the node that would spawn the session, and an unusable path SHALL be rejected with a cause naming the path and the condition. The node's path judgement SHALL include its workspace-root containment: the answer SHALL carry whether the path is within the node's workspace root alongside the existence and directory facts. A judgement answer that predates the containment field SHALL be treated as in scope, so an upgraded control plane keeps working against not-yet-upgraded nodes; the containment guarantee for remote registration holds only against upgraded nodes.

#### Scenario: path is judged where the spawn happens

- **WHEN** the control plane asks a node to start a session in a project directory
- **THEN** the node validates the path on its own filesystem before spawning
- **AND** a missing or unusable path is rejected with a cause rather than spawning elsewhere

#### Scenario: the judgement carries workspace containment

- **WHEN** the control plane asks a node to judge a project path
- **THEN** the answer states whether the path lies within the node's workspace root, and an out-of-scope path is rejected for registration

#### Scenario: a pre-containment answer stays compatible

- **WHEN** a node that does not report the containment field answers a path judgement
- **THEN** the control plane treats the answer as in scope and proceeds as before

#### Scenario: the same path on two hosts is not the same project

- **WHEN** two nodes both hold a directory at the same path
- **THEN** those are two distinct project references and neither is treated as the other
