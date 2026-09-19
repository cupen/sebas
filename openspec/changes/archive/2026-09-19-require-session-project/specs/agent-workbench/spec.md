## ADDED Requirements

### Requirement: Sessions belong to a project

Every session presented by the webui SHALL belong to a registered project. Creating a session SHALL require a target project: `POST /api/sessions` without a usable `project_id` (omitted, `null`, blank, or unknown) SHALL be rejected with a typed 400 naming `project_id` — no project-less session SHALL ever come into existence through any path, and no Inbox group SHALL be reintroduced. A session's project ownership SHALL be part of its identity: a spawn failure SHALL NOT detach the session from its project or from the agent that was asked to serve it. The one exception is Feishu-originated sessions, which have no project directory by nature and are presented by their originating surface (see the `feishu-option` capability).

#### Scenario: Creation without a project is rejected

- **WHEN** a create-session request omits `project_id`, sends `null`, sends a blank value, or names an unknown project
- **THEN** the request is rejected with HTTP 400 and an error message naming `project_id`
- **AND** no session, mapping, or placeholder is created

#### Scenario: Creation with a project binds every layer

- **WHEN** a create-session request names a registered project
- **THEN** the session's working directory and execution node are resolved server-side from that project's registration
- **AND** the created session appears under that project in the workbench rail

#### Scenario: A failed spawn keeps the session's ownership

- **WHEN** a session's agent spawn fails (unknown agent, missing binary, handshake failure)
- **THEN** the session's mapping keeps its project directory and requested agent kind/mode
- **AND** the session remains listed under its project in the rail with the failure state
- **AND** the failure reason is presented inline in the workbench

#### Scenario: Project-less persisted sessions are dropped

- **WHEN** the session state is restored or dumped and an entry carries no project directory on a non-Feishu channel
- **THEN** that entry is dropped with a warning rather than loaded or persisted
- **AND** internal archive records (`closed-*`, acp-session-mapping) are exempt and keep their original mapping's project identity

## MODIFIED Requirements

### Requirement: History group is the archive

The History group SHALL contain only archived sessions, listed newest-first by archive time. The rail SHALL render one group per registered project and SHALL NOT render an Inbox group: every session the webui presents belongs to a project, and the only project-less sessions are Feishu-originated ones, which remain out of the rail entirely and are presented by their originating surface. The History group SHALL show the total count of archived sessions and be collapsible.

#### Scenario: History holds only archived sessions

- **WHEN** the History group is expanded
- **THEN** every session listed has been archived

#### Scenario: History is sorted newest-first

- **WHEN** sessions are archived at different times
- **THEN** the History group lists them in descending order of archive time

#### Scenario: Inbox for unbound sessions

- **WHEN** a session has no project directory
- **THEN** the only such sessions are Feishu-originated ones: it appears in no rail group — the Inbox group does not exist — and History does not list it either; every webui-created session belongs to a project and is listed under it
