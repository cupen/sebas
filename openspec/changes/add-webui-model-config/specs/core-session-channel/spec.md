## ADDED Requirements

### Requirement: Provider management over the channel

The channel SHALL carry provider management requests — create/update, delete,
model-catalog probe, and agent-default selection — as additive message types
alongside the existing set. The core SHALL apply each request to its state
store (the single write authority) and reply with the resulting provider view
or a typed rejection naming the cause. Detached clients SHALL obtain provider
mutation capability through these requests and SHALL NOT write provider state
by any other route. Legacy clients that do not send the new message types
SHALL be unaffected.

#### Scenario: detached mutation is applied by the core

- **WHEN** a detached WebUI sends a provider update over the channel and then
  requests the provider snapshot
- **THEN** the core applied the change to its state store and the snapshot
  reflects it

#### Scenario: invalid request is typed-rejected

- **WHEN** a client sends a provider create missing its required name
- **THEN** the response is a typed rejection naming the cause, and no provider
  is created

#### Scenario: legacy client unaffected

- **WHEN** an old client that never sends provider messages operates the
  channel
- **THEN** all existing requests and events behave exactly as before the
  message set was extended
