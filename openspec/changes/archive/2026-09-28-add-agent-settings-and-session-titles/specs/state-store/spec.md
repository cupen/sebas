## MODIFIED Requirements

### Requirement: State methods on the core channel

The core channel SHALL expose state methods for snapshot queries (domains `providers` — projected with their model aliases — `settings`, `projects`, `presets`, `sessions`, `router_activity`, and `agents`) and mutations (provider/alias/settings/projects/agents CRUD; `aliases` is its own mutation domain), plus a change subscription that delivers a notification after each committed mutation. Access SHALL be governed by the channel's authentication; unauthorized peers are denied.

#### Scenario: Snapshot reflects committed mutation

- **WHEN** a client performs an alias mutation and then requests a providers snapshot
- **THEN** the snapshot contains the new alias

#### Scenario: Agents snapshot reflects agent mutation

- **WHEN** a client performs an agents mutation (create, update, or delete) and then requests an agents snapshot
- **THEN** the snapshot contains the resulting agent rows

#### Scenario: Subscribers are notified after commit

- **WHEN** a provider mutation commits
- **THEN** subscribed clients receive a change notification scoped to providers

#### Scenario: Unauthorized peer denied

- **WHEN** an unauthenticated peer calls a state method
- **THEN** the request is denied with an authorization error
