## MODIFIED Requirements

### Requirement: Turn content retrieval

The channel SHALL provide a method returning the rendered turn content the core
holds for a given session, so a client can display an agent conversation it did
not itself receive. The response SHALL carry a monotonic position so a client can
request only what it has not yet seen.

Each entry SHALL state its role in the conversation — a submission by the
operator (`kind = "prompt"`) or content produced by the agent
(`kind = "content"`) — and its render type (`element_type`): plain content
(`markdown`), `thinking`, a tool invocation (`tool`), or an error (`error`).
Submission entries SHALL be delivered as part of the content; a client SHALL NOT
have to reconstruct the operator's turns from a separate field, and SHALL be able
to tell a tool invocation apart from the agent's prose.

#### Scenario: incremental turn fetch

- **WHEN** a client requests turn content for a session with a position it has
  already seen
- **THEN** the response contains only content after that position, with a new
  position to use next time

#### Scenario: submission and tool entries are distinguishable

- **WHEN** a client requests turn content for a session in which the operator
  submitted a message and the agent replied with prose, thinking and a tool call
- **THEN** the operator's submission arrives as its own entry, and the tool
  invocation is distinguishable from the prose rather than looking identical to it
