## REMOVED Requirements

### Requirement: Standalone detached semantics

**Reason**: The behavior this requirement documents is the defect this change
removes. It specified that the watchdog-spawned WebUI operates on its own
`RouterHandle` restored from the state file with the outbound channel dropped —
which, combined with the state file being written only at core shutdown, means
the production WebUI shows the session set from the previous core exit and
mutates nothing. Replaced by "Standalone core-client semantics" below.

## ADDED Requirements

### Requirement: Standalone core-client semantics

The watchdog-spawned (standalone) WebUI SHALL obtain session data and perform
session mutations exclusively through the core session channel, and SHALL NOT
construct its own `RouterHandle`, restore session state from the state file, or
hold a throwaway session manager. Session create, message send, and close SHALL
be requests to the core that spawn real ACP sessions and take effect in the
running core. The in-process `run --webui` path SHALL use an equivalent
in-process backend so that both paths present the same behavior to the browser.

#### Scenario: standalone message send reaches the core

- **WHEN** the user sends a message through a standalone WebUI's session page
- **THEN** the request is delivered to the core, which applies it to the real
  session, and the change is observable in the core rather than only in the
  WebUI process

#### Scenario: standalone board is live

- **WHEN** the core creates or updates a session while a standalone WebUI page is
  open
- **THEN** the WebUI reflects the change without a manual reload, and never
  renders a session set reconstructed from the state file

#### Scenario: both paths behave alike

- **WHEN** the same page is rendered under `sebas webui` and under
  `run --webui`
- **THEN** session data and the availability of session controls are equivalent,
  differing only in which backend implementation serves them

### Requirement: Session backend seam

The WebUI crate SHALL access sessions through a backend abstraction rather than a
concrete `RouterHandle`, in the same shape as the existing admin adapter, so the
crate carries no knowledge of whether the core is in-process or across a socket.
The crate SHALL NOT depend on the sebas binary crate to obtain a backend; the
binary crate SHALL supply the implementation at startup.

#### Scenario: WebUI is testable without a core

- **WHEN** the WebUI's route tests run
- **THEN** they drive routes through a fake backend, with no ACP child, no socket,
  and no state file

#### Scenario: no backend leaks into templates

- **WHEN** a page is rendered
- **THEN** which backend is in use is not visible in the markup except where the
  channel's degradation contract requires stating that the core is not connected
