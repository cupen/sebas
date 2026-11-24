# agent-task-center Specification

## Purpose

Provides a project-scoped task collaboration layer that lets human operators and agents claim, execute, and review work together. A task is a beads issue stored in the project's `.beads/` repository-local database; sebas supplies a thin wrapper (HTTP API + WebSocket push + workbench panel) and does not reimplement beads' claim, lease, journal, or comment semantics.

## ADDED Requirements

### Requirement: Task lifecycle and state machine

A task SHALL progress through the states `open → in_progress → in_review → done`, with `cancelled` as a terminal escape. `open → in_progress` SHALL happen via a claim; `in_progress → in_review` SHALL happen via the session-terminal hook (see「Session-terminal hook advances task」) or an explicit operator action; `in_review → done` SHALL be restricted (see「Close is gated to humans and the designated reviewer」). The state machine SHALL be expressed in beads' native status vocabulary plus an `in-review` label: `open`/`in_progress` are beads statuses, `in_review` is the presence of the `in-review` label on an `in_progress` issue, `done` is `bd close`, and `cancelled` is `bd close --reason cancelled`.

#### Scenario: New task enters the public queue

- **WHEN** an operator creates a task without specifying an assignee
- **THEN** the task exists as a beads issue with status `open`, no assignee, and no `in-review` label, and appears in the project's unassigned queue

#### Scenario: Claim moves task to in_progress

- **WHEN** an operator or agent claims an `open` task
- **THEN** the issue's status becomes `in_progress`, the assignee is set to the claiming principal, and the task leaves the public queue

#### Scenario: Entering review adds the in-review label

- **WHEN** a task is moved from `in_progress` to review (by the session-terminal hook or by an explicit operator action)
- **THEN** the beads issue gains the `in-review` label and its status remains `in_progress`

#### Scenario: Done removes the issue from the active board

- **WHEN** a human or the designated reviewer closes an in-review task
- **THEN** the beads issue is closed via `bd close`, the `in-review` label no longer matters, and the task appears in the collapsed "Done / Cancelled" region

#### Scenario: Cancellation records the reason

- **WHEN** an operator cancels a task in any non-terminal state
- **THEN** the beads issue is closed via `bd close --reason cancelled`, and the task is shown as cancelled in the collapsed region

### Requirement: Claim is atomic and delegated to beads

Claim SHALL be implemented as `bd update <id> --claim`, inheriting beads' atomic claim semantics (single-winner, idempotent for the same actor, refuses to overwrite another actor's live `in_progress` claim). The WebUI SHALL surface beads' claim outcome verbatim: success returns the updated task, losing the race returns 409 with the current holder named, claiming an already-owned task returns 200 idempotently.

#### Scenario: Two actors racing a claim

- **WHEN** two actors (human or agent) call claim on the same `open` task concurrently
- **THEN** exactly one succeeds, the loser receives 409 naming the winner, and the beads issue shows a single assignee

#### Scenario: Re-claim by the same actor is idempotent

- **WHEN** an actor claims a task they already hold
- **THEN** the response is 200 with the current task state and no state change is recorded

#### Scenario: Release returns the task to the queue

- **WHEN** the current assignee releases a task
- **THEN** `bd unclaim <id>` is invoked, the assignee becomes empty, the status returns to `open`, and the task reappears in the public queue

### Requirement: Assignee vocabulary is validated at the WebUI boundary

The assignee field SHALL be one of: a registered WebUI username, or a configured agent kind (the set exposed by `/api/agents`). The WebUI SHALL validate the assignee before invoking `bd update --assignee` and SHALL reject unknown principals with 400. The same vocabulary applies to the `reviewer` field stored in issue metadata.

#### Scenario: Unknown assignee is rejected

- **WHEN** an operator submits an assign or claim request naming an unregistered user or unknown agent kind
- **THEN** the WebUI returns 400 with the unrecognized principal named and no `bd` invocation occurs

#### Scenario: Agent kind is a valid assignee

- **WHEN** an operator assigns a task to `claude` and `claude` is a configured agent
- **THEN** the assignee is accepted, the beads issue records `claude` as assignee, and the task is shown as held by that agent kind

### Requirement: Close is gated to humans and the designated reviewer

`bd close <id>` SHALL only be invocable through the WebUI by a human operator or by the agent kind named in the issue's `reviewer` metadata field (set at task creation). Any other agent kind calling close SHALL receive 403. Humans bypass the reviewer restriction.

#### Scenario: Reviewer agent may close

- **WHEN** the issue's reviewer metadata names `claude` and `claude` calls close
- **THEN** the close proceeds and the task transitions to done

#### Scenario: Non-reviewer agent is refused

- **WHEN** the issue's reviewer metadata names `claude` and `codex` calls close
- **THEN** the WebUI returns 403 and the beads issue is not modified

#### Scenario: Human always may close

- **WHEN** any authenticated human operator calls close on an in-review task
- **THEN** the close proceeds regardless of the reviewer metadata

### Requirement: Beads repository initialization is lazy and automatic

On the first access to a project's task panel, if the project directory has no `.beads/` database, the WebUI SHALL run `bd init --prefix <first 4 chars of project directory basename>` followed by `bd config set events-journal true` in that directory. If `.beads/` already exists, the WebUI SHALL only ensure `events-journal = true` and SHALL NOT reinitialize. Initialization failure SHALL surface as an inline error in the task panel, not as a panic.

#### Scenario: First visit initializes beads

- **WHEN** an operator opens the task panel of a project that has never had a `.beads/` directory
- **THEN** the panel renders after `bd init` and `bd config set events-journal true` complete successfully

#### Scenario: Existing beads repository is preserved

- **WHEN** an operator opens the task panel of a project that already has `.beads/` (possibly created by the user manually)
- **THEN** the existing database is used as-is and only `events-journal` is forced on

#### Scenario: Init failure is stated inline

- **WHEN** `bd init` fails (e.g., `bd` not installed, project directory not writable)
- **THEN** the task panel displays the failure cause and no tasks are listed, and the failure does not crash the WebUI

### Requirement: Task board is a three-column kanban

The task panel SHALL render three columns — `Unassigned`, `In progress`, `In review` — plus a collapsed `Done / Cancelled` region. Each column header SHALL show its task count. Cards SHALL support drag-and-drop between columns; a drag SHALL trigger the corresponding state transition (claim, release, add in-review label, close) via the existing HTTP API. Tasks in the `Done / Cancelled` region SHALL NOT be draggable.

#### Scenario: Columns reflect the live state

- **WHEN** the panel renders
- **THEN** each task appears in the column matching its beads state, column counts are accurate, and the done/cancelled region is collapsed by default

#### Scenario: Drag triggers the state transition

- **WHEN** an operator drags a card from `Unassigned` to `In progress`
- **THEN** the WebUI calls claim on that task for the current operator, and on success the card appears in `In progress`

#### Scenario: Drag to done requires the close permission

- **WHEN** an agent-kind principal attempts to drag a card from `In review` to `Done` and is not the designated reviewer
- **THEN** the WebUI refuses the drag and shows the same 403 reason the close endpoint would return

### Requirement: Task card displays collaboration context

Each card on the board SHALL show: title (two-line truncation with tooltip), assignee (circular avatar with first-letter for humans, agent-type icon for agents, dashed circle for unassigned), comment count, session count (number of sessions linked via `session_map.task_id`), and a relative "last updated" timestamp. Agent-kind assignees SHALL additionally display a small `agent` label.

#### Scenario: Card fields are populated

- **WHEN** the board renders a card
- **THEN** all five fields are visible with the specified formatting, and long titles truncate with ellipsis plus tooltip

#### Scenario: Agent assignee is visually distinct

- **WHEN** a card's assignee is an agent kind
- **THEN** the avatar shows the agent-type icon (not a letter), and a small `agent` chip appears next to it

### Requirement: Task detail is a two-column page

The task detail page SHALL render a left main column (title, markdown description, comment stream) and a right sidebar (metadata: status/assignee/reviewer/created_at; action buttons; linked-session list). Action buttons SHALL be conditionally rendered by the current state and the caller's identity. A "Start a session for this task" button SHALL always be present for authorized callers and SHALL open the existing session-creation dialog (titled "Advance task bd-XX: <title>") with `task_id` attached.

#### Scenario: Detail page layout

- **WHEN** an operator opens a task
- **THEN** the left column shows title/description/comments and the right sidebar shows metadata, actions, and the session list

#### Scenario: Action buttons follow state and identity

- **WHEN** the viewer is the assignee of an in_progress task
- **THEN** the action buttons are "Release" and "Enter review"; the close button is hidden

#### Scenario: Close is hidden from non-reviewer agents

- **WHEN** an agent that is not the designated reviewer views an in-review task
- **THEN** no "Mark done" button is rendered; only "Cancel" is available if permitted

#### Scenario: Start session opens the creation dialog

- **WHEN** an operator clicks "Start a session for this task"
- **THEN** the existing session-creation dialog opens with title "Advance task bd-XX: <title>" and confirming creates a session whose `session_map.task_id` is set

### Requirement: Task comment stream is beads-backed

The task detail page's comment stream SHALL be backed by `bd comment` / `bd comments`. Each comment SHALL display author (with the same human/agent avatar convention as the assignee), timestamp, and markdown body. Agents MAY post comments via the same endpoint the WebUI exposes to humans; the comment author SHALL be the calling principal.

#### Scenario: Posting a comment

- **WHEN** an operator or agent posts a comment on a task
- **THEN** the comment is recorded via `bd comment <id>` with the caller as author and appears in the stream

#### Scenario: Comment stream displays author identity

- **WHEN** the stream renders comments from both a human and an agent
- **THEN** each comment shows the correct avatar type, and agent comments carry the `agent` chip

### Requirement: Real-time board refresh via WebSocket push

The WebUI backend SHALL poll `bd events tail --since <last_seq>` every 300ms with a 200ms per-call timeout, for each project with an active task panel subscriber. New events SHALL be pushed to subscribed WebSocket clients as `task_updated` messages carrying the full beads issue snapshot from the journal event. The frontend SHALL apply each event as a state replacement for the corresponding task (no `bd show` re-fetch). The polling loop SHALL track `last_seq` per project and SHALL NOT miss or duplicate events.

#### Scenario: New event triggers a push

- **WHEN** a mutation commits to the beads DB and the next poll observes it
- **THEN** every subscribed client receives a `task_updated` message with the issue snapshot, and the board updates without a full refetch

#### Scenario: End-to-end latency is sub-second

- **WHEN** a mutation is committed to the beads DB
- **THEN** the corresponding `task_updated` event reaches subscribed clients within 1 second at p95 (spike-measured: p95 = 330ms at 200ms poll interval; production target: ≤ 500ms at 300ms poll interval)

#### Scenario: Polling recovers from a transient failure

- **WHEN** a single `bd events tail` call fails or times out
- **THEN** the polling loop logs the failure and continues with the same `last_seq` on the next tick; no events are skipped once `bd` recovers

### Requirement: Linked session list shows the task's execution history

The task detail page SHALL list every session whose `session_map.task_id` equals the task's beads issue id, ordered by `created_at DESC`. Each entry SHALL show the session title, bound agent kind, current phase, and created_at; clicking an entry SHALL switch the stage to the conversation view focused on that session.

#### Scenario: Sessions are listed newest-first

- **WHEN** a task has been advanced by three sessions at different times
- **THEN** the sidebar lists all three, newest first, each with title/agent/phase/timestamp

#### Scenario: Clicking a session jumps to the conversation

- **WHEN** an operator clicks a linked session entry
- **THEN** the stage switches to the conversation view focused on that session

### Requirement: Session-terminal hook advances task

When a session with a non-empty `task_id` enters a terminal phase (`Closed` or `Exited`) AND the session's last completed turn finished normally (`stop_reason = end_turn`, not `cancelled`, not an error), the WebUI backend SHALL invoke `bd update <task_id> --add-label in-review` for the linked task. Sessions ending in `Failed` or `Terminated` SHALL NOT trigger the label. The hook SHALL only fire for the task's most-recently-created session (older sessions terminating MUST NOT retroactively change task state).

#### Scenario: Normal completion advances the task to review

- **WHEN** the most recent session for a task reaches `Closed` after a successful turn
- **THEN** the linked task gains the `in-review` label and moves to the `In review` column

#### Scenario: Failed session does not advance

- **WHEN** the most recent session for a task reaches `Failed`
- **THEN** the task remains `in_progress` with no `in-review` label

#### Scenario: Manually closed session without a successful turn does not advance

- **WHEN** the most recent session for a task is closed by the operator before any turn completes normally
- **THEN** the task remains `in_progress`

#### Scenario: Older session terminating does not affect task state

- **WHEN** a task has two linked sessions A (older) and B (newer, still active), and A reaches `Closed`
- **THEN** the task state is unchanged; only B's terminal transition matters

### Requirement: Single-machine multi-agent scope (v1)

The first version SHALL support a single-machine deployment where all agents and the WebUI share one `.beads/` repository. Cross-machine sync via `bd dolt push` SHALL NOT be integrated by sebas in this version; users who sync manually MUST re-baseline (refresh full task list) after a sync, because the events journal is per-replica and does not carry remote mutations.

#### Scenario: Cross-machine sync requires re-baseline

- **WHEN** a user runs `bd dolt pull` in the project directory
- **THEN** the WebUI SHALL provide a manual "Refresh tasks" affordance that performs a full `bd list --json` reload and resets `last_seq` to the journal's current head

#### Scenario: Single-machine multi-agent works without re-baseline

- **WHEN** all mutators (WebUI and agents) operate on the same `.beads/` repository
- **THEN** the events journal captures every mutation and the polling loop delivers them all
