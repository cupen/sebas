## ADDED Requirements

### Requirement: Remote session lifespan is bound to its node

A session placed on an execution node SHALL keep running while the control plane is absent: a dropped link SHALL NOT terminate it, and a control-plane restart SHALL NOT terminate it. The session's life SHALL instead be bound to the node that runs it — a node restart terminates the sessions it was running, and the control plane SHALL report them as terminated rather than as still running. Reconciliation SHALL NOT create a replacement session for one that is still alive on its node.

#### Scenario: control-plane restart does not end remote sessions

- **WHEN** the control plane restarts while sessions are active on a node
- **THEN** those sessions keep running on the node
- **AND** after reconnection the control plane reports them as the same sessions, with their history continued rather than restarted

#### Scenario: link loss does not end remote sessions

- **WHEN** the websocket between the control plane and a node drops and is re-established
- **THEN** no session on that node is terminated or replaced

#### Scenario: node restart ends its sessions

- **WHEN** a node process restarts
- **THEN** the sessions it was running are reported as terminated with the node named as the cause
- **AND** the control plane does not present them as live or as silently resumable

#### Scenario: terminated is not recreated

- **WHEN** the control plane reconciles with a node that no longer holds a session it previously knew
- **THEN** that session is marked terminated and no new session is created in its place
