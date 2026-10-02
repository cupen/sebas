## ADDED Requirements

### Requirement: Rename a project

The workbench SHALL let the operator rename a registered project from the project row menu. The backend SHALL expose `POST /api/projects/{id}/rename` accepting `{ name }`: a non-empty name SHALL be persisted in the project registry (SQLite, surviving restart) and the rail SHALL reflect the new name without a page reload; an empty or whitespace-only name SHALL be rejected with a 400 and the dialog SHALL surface the error inline. Renaming SHALL NOT change the project's path, sort order, node attribution, or its sessions' attribution.

#### Scenario: rename via rail menu

- **WHEN** the operator picks "重命名" in a project row menu, enters a new name, and confirms
- **THEN** the rail row shows the new name immediately and the name is persisted across a core restart

#### Scenario: empty name rejected

- **WHEN** the operator submits a rename with an empty or whitespace-only name
- **THEN** the request fails with a 400 and the dialog shows the inline error without closing

#### Scenario: rename leaves sessions attributed

- **WHEN** a project with existing sessions is renamed
- **THEN** its sessions remain listed under the project and keep their history, keys, and unread state
