# feishu-reactions Specification

## Purpose
Defines the emoji-reaction state machine attached to the user's Feishu
messages: the acknowledgment reaction on inbound messages, phase transitions
during a turn (seed → working → done), swap semantics when the reaction
changes, target-message selection, and the cases where no reaction is
emitted.

## Requirements

### Requirement: Reaction vocabulary

Reactions SHALL use Feishu `emoji_type` tokens rather than raw Unicode
emoji: `Get` (👌 acknowledgment), `OnIt` (🚧 working), `DONE` (✅ finished),
`CrossMark` (❌ failed) — the Feishu API rejects raw Unicode emoji with
error 231001. Phase reactions are emitted by the IM service's frontend as it
observes the core's session phase (see `im-service`), not by the core process.

#### Scenario: token vocabulary

- **WHEN** the IM service applies a phase reaction
- **THEN** the react API call carries a Feishu `emoji_type` token
  (`Get`/`OnIt`/`DONE`), not a Unicode character

### Requirement: Inbound acknowledgment

Every inbound user text or media message that passes filtering SHALL receive
a one-shot `Get` (👌) acknowledgment reaction on that user message before
processing begins. Acknowledgment reactions are tracked per message id
(`record_ack`/`is_acked`), separately from the session's phase-reaction
tracker; a message that already carries the ack (e.g. a duplicate delivery)
SHALL NOT be re-acked. The ack is NEVER removed: terminal reactions (`DONE`
on the card, or `CrossMark` on the user message at terminal error) are
overlaid alongside it as independent receipts, not swapped for it. The IM
service's frontend SHALL apply this ack; the core process
SHALL NOT emit an ack reaction for IM channels.

#### Scenario: text message acknowledged

- **WHEN** the user sends a text message to an active chat
- **THEN** the IM service reacts `Get` on that message before the turn's
  streaming begins

#### Scenario: ack stays while phases overlay

- **WHEN** a message carrying the 👌 ack transitions through working to done
- **THEN** the ack reaction is never removed; phase/terminal reactions land on
  their own targets (the card, or the user message for terminal failure) as
  independent receipts

### Requirement: Phase state machine

The session's reaction phase SHALL start at seed (`Get`). Streaming events
(text, thinking, tool start/progress/end, and non-terminal errors) transition
seed → working (`OnIt`) exactly once; once working, further streaming events
do not change the reaction. `Finished` transitions to `DONE` (✅) regardless
of the current phase. Permission requests and usage updates never change the
reaction. There is no transition back to seed. The core reports the phase via
the session snapshot (`SessionInfo.phase`); the IM service's frontend SHALL
drive the reaction from phase changes it observes.

#### Scenario: seed to working once

- **WHEN** a tool-start event arrives while the phase is seed, followed by
  more streaming events while working
- **THEN** exactly one `OnIt` reaction is applied and subsequent streaming
  events emit none

#### Scenario: finished from any phase

- **WHEN** the turn finishes while the phase is still seed (no streaming
  event preceded it)
- **THEN** the reaction transitions directly to `DONE`

#### Scenario: done to working on continuation

- **WHEN** the user continues a finished session and streaming resumes
- **THEN** the reaction flips from `DONE` back to `OnIt`

### Requirement: Swap semantics

A reaction change SHALL be planned against the session's current recorded
reaction: same emoji → no API call; different emoji → remove the old
reaction then add the new one; no current reaction → add only. Removal of
the old reaction is best-effort (failure logs a warning and the add still
proceeds); failure to add the new reaction propagates as an error.

#### Scenario: no-op when unchanged

- **WHEN** a phase transition computes the same emoji already recorded
- **THEN** no react or unreact API call is made

#### Scenario: unreact failure tolerated

- **WHEN** the un-react API call fails during a swap
- **THEN** the new reaction is still applied and only a warning is logged

### Requirement: Target message selection

Phase reactions SHALL target the session's root card message
(`card_msg_id`); when no card message id is known, the reaction is silently
skipped. (Rationale: the IM frontend renders the card it owns; targeting the
card is the deployment reality after extract-im-service.)

#### Scenario: card target

- **WHEN** a session with a rendered card finishes a turn
- **THEN** the `DONE` reaction lands on that card message

#### Scenario: no target skipped

- **WHEN** a phase change is observed for a session with no card message id
- **THEN** no reaction API call is made and no error surfaces

### Requirement: Terminal states

A finished turn SHALL emit the `DONE` (✅) reaction on the card. A
terminal error SHALL additionally mark the triggering user message with the
`CrossMark` (❌) reaction (the card itself still carries no ❌ reaction —
card flips only rewrite the card face), so the failed turn is visible even
when the card arrived late or was never rendered; a late-arriving card SHALL
still be back-filled with the terminal mark.

#### Scenario: finished emits done

- **WHEN** a turn completes successfully
- **THEN** a `DONE` reaction is applied to the session's card

#### Scenario: terminal error marks the user message

- **WHEN** the ACP session reports a terminal error
- **THEN** the triggering user message receives the `CrossMark` reaction and
  the card body carries the ❌ error row

### Requirement: Permission wait keeps reaction

While a turn is suspended awaiting a permission decision, the reaction SHALL
remain unchanged (typically `OnIt`); permission request events never trigger
a reaction transition.

#### Scenario: permission mid-turn

- **WHEN** a permission request arrives while the phase is working
- **THEN** the `OnIt` reaction stays in place through the wait
