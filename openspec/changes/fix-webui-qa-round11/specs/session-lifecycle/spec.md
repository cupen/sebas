## ADDED Requirements

### Requirement: Native-backed sessions report lifecycle truth

A session driven by the native agent kernel SHALL advance the shared session-state vocabulary exactly like ACP-backed sessions do — through the same metadata channel, with status derivation remaining the single shared function. The native path SHALL publish, at minimum: the turn-in-flight truth while a native turn is streaming or parked on a pending permission request; the terminal transition when a turn settles (done for a completed turn, failed for an errored one); and the session title derived from the operator's first message. Consequences that MUST hold for every observer (rail status dot, history cards, active/dormant counters, composer control): a native session that finished a turn SHALL NOT present as queued or active; the top-level active/active-vs-dormant counts SHALL reflect native sessions truthfully; and a native session SHALL NOT remain unnamed («未命名会话») after its first message when ACP sessions in the same view are named.

#### Scenario: native turn reports in-flight while streaming

- **WHEN** a native session is streaming a turn (e.g. a long trickle scenario)
- **THEN** the session reports turn-in-flight truth on the shared channel
- **AND** the composer shows the stop affordance and the rail dot shows the working status

#### Scenario: native turn settles to done

- **WHEN** a native turn completes normally
- **THEN** the session's status advances to done on the rail and history cards
- **AND** the active/dormant counters no longer count it as active

#### Scenario: native turn settles to failed

- **WHEN** a native turn ends in an upstream error
- **THEN** the session's status advances to failed, not queued

#### Scenario: native session is named after the first message

- **WHEN** the operator sends the first message in a new native session
- **THEN** the rail and history present the session titled from that message, matching the ACP naming behavior

#### Scenario: counts include native sessions truthfully

- **WHEN** the history page renders with a mix of ACP and native sessions, of which some native turns have completed
- **THEN** the active and dormant counters agree with the per-session statuses shown on the same page
