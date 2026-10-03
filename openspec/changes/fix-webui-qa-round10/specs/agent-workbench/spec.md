## ADDED Requirements

### Requirement: Rail history badge reflects live session counts

The rail's history (archive) entry SHALL display the same session statistic the sessions view uses, and SHALL update when sessions are created, closed, or archived — it MUST NOT remain a static zero while the sessions view reports non-zero totals.

#### Scenario: Badge tracks session creation

- **WHEN** the operator creates a session while the history badge shows a count
- **THEN** the rail statistic that counts sessions reflects the new session without a full reload

### Requirement: Live transcript never blank-paints

During a running turn, the transcript panel SHALL keep rendering its entries: a streaming or follow-up turn after an oversized (wide-table) turn MUST NOT leave the transcript visually blank while the underlying entries exist. If entries are present in the document, they SHALL be visible without a reload.

#### Scenario: Streaming turn after a wide-table turn

- **WHEN** a session contains a turn with an oversized wide table, and the operator submits another prompt that streams output
- **THEN** the transcript shows the streaming content as it arrives (screenshot-verifiable), with no full-panel blank state
- **AND** the page requires no reload to make the entries visible

### Requirement: Oversized content stays inside the transcript container

The transcript container SHALL NOT be stretched by oversized content (wide tables, long code lines): the conversation surface keeps its layout width regardless of entry content. Horizontally scrollable regions inside entries (wide tables, code blocks) SHALL present a visible scrollbar affordance.

#### Scenario: Wide table does not stretch the conversation

- **WHEN** an assistant turn renders a GFM table wider than the transcript panel
- **THEN** the transcript container width stays at its layout width (no hidden thousands-of-pixels overflow)
- **AND** the table scrolls horizontally inside its own region with a visible scrollbar

### Requirement: Turn model badge is frozen at observation time

A transcript entry's model badge SHALL render the model observed for that turn (the value carried by the turn's frames), and SHALL NOT be rewritten when the session's current model changes later. Historical entries keep the model they ran under.

#### Scenario: Switching models does not rewrite history

- **WHEN** earlier turns ran under model A and the operator then switches the session to model B
- **THEN** the earlier entries still display model A and only new turns display model B
