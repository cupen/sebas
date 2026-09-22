## ADDED Requirements

### Requirement: Protocol version negotiation on the channel handshake

The channel handshake SHALL carry the protocol version each side speaks, so that a version mismatch is detected and reported instead of surfacing as a decode failure or a silently misinterpreted request. The version field SHALL be additive: a peer that omits it SHALL be treated as speaking the first version, and a peer that receives a version field it does not recognize SHALL reject the connection with a typed, honest rejection naming both versions rather than proceeding. Version negotiation SHALL NOT weaken the existing authentication order: peer-uid checks on Unix and the shared-secret check SHALL still be performed before any request is processed.

#### Scenario: A peer that omits the version is treated as the first version

- **WHEN** a client connects without sending a version
- **THEN** the core treats it as speaking version 1 and serves it normally

#### Scenario: A future version is rejected with both versions named

- **WHEN** a client announces a protocol version this build does not support
- **THEN** the connection is rejected with a typed rejection naming the client's version and the supported one
- **AND** no request on that connection is processed

#### Scenario: A version field is ignored by an older peer

- **WHEN** a client that does not know the version field connects to a core that sends it
- **THEN** the client decodes the handshake successfully because the field carries a default
- **AND** the existing authentication behavior is unchanged

#### Scenario: Authentication still precedes version handling

- **WHEN** a connection supplies a wrong secret together with a version this build does not support
- **THEN** it is closed as an authentication failure without disclosing version support
- **AND** no version rejection is sent
