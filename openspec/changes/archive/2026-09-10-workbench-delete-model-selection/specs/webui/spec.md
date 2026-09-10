## ADDED Requirements

### Requirement: Creation-time model applies to ACP sessions

The `model` field of `POST /api/sessions` SHALL be honored for ACP-backend sessions: when present, the backend SHALL deliver it to the spawned ACP child as the session's model configuration before the first prompt runs, so the first turn already uses the chosen model. For agents that expose no model configuration surface, the field SHALL remain a silent no-op (the session uses its default model and no model UI is shown), consistent with existing behavior. A model id the agent rejects SHALL surface as a typed error on the session rather than a silent fallback.

#### Scenario: chosen model applies from the first turn

- **WHEN** the operator creates an ACP session with a `model` the agent supports
- **THEN** the session's first turn runs with that model, and the session's `current_model` afterwards reflects it

#### Scenario: agent without a model surface ignores the field

- **WHEN** the operator creates an ACP session with a `model` against an agent that exposes no model configuration
- **THEN** the session spawns with the agent's default model and no error is raised

#### Scenario: rejected model is a typed error

- **WHEN** the operator creates an ACP session with a `model` the agent rejects
- **THEN** the session surfaces a typed rejection and does not silently fall back to the default model

## MODIFIED Requirements

### Requirement: Session dashboard and focus semantics

The cross-project session list SHALL render one row per known session (encoded key, chat and thread ids, session id, status, phase, relative last-active), active-first, and SHALL be reachable from the workbench rather than from primary navigation. The session list SHALL exclude archived sessions — those are served by `GET /api/archive`. Visiting a session's detail page or posting `/switch` SHALL set the webui-side focused session — a display pointer only that never changes message routing — and `switch` returns the redirect target or 404 for an unknown key. Switching the displayed project SHALL NOT alter the focused session pointer.

The focused-session pointer SHALL be the single source of truth for the workbench composer's follow-up vs creation mode: with a focused session the composer targets that session; without one the composer is in creation mode. Any path that focuses a session (switch endpoint, deep-link visit, placeholder creation) SHALL leave the composer able to submit a follow-up message to that session without further operator action.

#### Scenario: focus is cosmetic

- **WHEN** the user focuses session B in the WebUI while session A is active
- **THEN** subsequent Feishu messages still route per the router's own session mapping, unchanged

#### Scenario: switch unknown key

- **WHEN** `/api/sessions/{key}/switch` posts a key not in the map
- **THEN** the response is 404

#### Scenario: project switch leaves focus alone

- **WHEN** the operator switches the displayed project
- **THEN** the focused session pointer is unchanged

#### Scenario: archive hides session from list

- **WHEN** a session is archived
- **THEN** it is no longer returned by `GET /api/sessions` and appears only in the `GET /api/archive` response

#### Scenario: focusing enables immediate follow-up

- **WHEN** a session becomes focused through any supported path
- **THEN** the workbench composer's next submission is delivered to that session as a follow-up message
