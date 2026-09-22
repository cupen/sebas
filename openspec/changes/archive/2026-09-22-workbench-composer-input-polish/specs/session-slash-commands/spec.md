## MODIFIED Requirements

### Requirement: Composer renders an incrementally-filtered command palette

When the composer input's first character is `/`, the composer SHALL render a command palette above the input listing the session's advertised commands — each row showing the command name and its argument hint when available, kept to a single compact line. A command's description SHALL NOT be rendered inline in the row: it SHALL be presented in a floating detail bubble that appears when the row is hovered or made the keyboard highlight target. The bubble SHALL render the description as sanitized markdown, SHALL be bounded in size (width and height caps well below the viewport) and scroll internally when the description exceeds those bounds, and SHALL disappear when the hover or highlight moves away, the palette closes, or the session's command surface is absent. Keyboard palette navigation (↑/↓) SHALL keep the bubble anchored to the highlighted row with the same immediacy as hover, so the palette is fully usable without a pointer. The palette SHALL support ↑/↓ selection, Esc to dismiss, and a two-phase Enter/Tab: with a highlighted entry, the first Enter/Tab completes the command (inserts `name + space`, keeps focus in the input for arguments) and a subsequent Enter submits; Enter with no highlighted entry submits the raw text directly. The palette SHALL NOT render for sessions without a command surface.

#### Scenario: palette filters as the operator types

- **WHEN** the operator types `/`, then continues with `co` in a session advertising `compact` and `goal`
- **THEN** the palette appears on `/` listing both commands and narrows to `compact` as `co` is typed

#### Scenario: description renders only in the hover bubble

- **WHEN** the command palette is open and the operator hovers a command row
- **THEN** the row itself shows only the command name and argument hint, and the description appears as a bounded, internally scrollable markdown bubble anchored to that row

#### Scenario: keyboard highlight shows the same bubble

- **WHEN** the palette is navigated with ↑/↓ so that a row becomes the keyboard highlight target
- **THEN** the detail bubble appears for that row exactly as it would on hover, and moving the highlight moves the bubble

#### Scenario: oversized description scrolls inside the bubble

- **WHEN** a command's markdown description exceeds the bubble's size caps
- **THEN** the bubble scrolls internally, never growing beyond its caps, and its content is sanitized before rendering

#### Scenario: two-phase completion

- **WHEN** the operator highlights `/goal` and presses Tab (or Enter)
- **THEN** the input becomes `/goal ` with the palette dismissed and focus retained; a subsequent Enter submits the message

#### Scenario: palette absent without command surface

- **WHEN** the focused session has an empty command list and the operator types `/`
- **THEN** no palette renders and the text stays ordinary input
