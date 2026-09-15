## ADDED Requirements

### Requirement: Rail selection hierarchy is visually distinct

The rail SHALL render the project-selection highlight and the focused-session
marker as visually distinct states: the focused-session marker SHALL keep the
accent treatment (accent background), while the selected project row SHALL
emphasize with a neutral treatment (elevated surface and brighter text, not
the accent color). When a project row and a session row are highlighted at
the same time — including the session focused inside the selected project —
the two states SHALL be distinguishable at a glance. The selected-project
semantics are unchanged: the highlight still names the project the workbench
is displaying.

#### Scenario: project selection and focused session are both visible

- **WHEN** the operator selects a project whose session is the focused one
- **THEN** the project row and the session row are both highlighted, and the two highlights use clearly different visual treatments rather than the same accent style

#### Scenario: focused-session marker keeps the accent

- **WHEN** a session becomes the focused session
- **THEN** its rail row keeps the accent-background marker regardless of any project selection state

#### Scenario: selected project uses neutral emphasis

- **WHEN** the operator selects a project row
- **THEN** the row is emphasized with a neutral surface and brighter text and is not rendered with the accent background used by the focused-session marker

### Requirement: Creating a session lands focus on the placeholder

Confirming the new-session dialog SHALL leave the target project's group
expanded (or expand it), make the new placeholder's rail row visible and
marked as the current session, and move keyboard focus into the composer
input so the operator can type the first message immediately. The creation
flow SHALL NOT collapse the project group it was opened from.

#### Scenario: project group stays expanded after creation

- **WHEN** the operator confirms the creation dialog opened from an expanded project row
- **THEN** the project group remains expanded and the new session row is visible under it

#### Scenario: placeholder row is selected

- **WHEN** the creation dialog is confirmed
- **THEN** the new placeholder's rail row is marked as the current session

#### Scenario: composer input has keyboard focus

- **WHEN** the creation dialog is confirmed and the workbench renders the placeholder
- **THEN** the composer text input holds keyboard focus and the first keystroke goes into it
