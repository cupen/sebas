## ADDED Requirements

### Requirement: Session usage accumulation survives agent restarts

The session head's cumulative token usage (total input / output) SHALL be owned
by the session's persisted record, not by the agent child's in-memory counter.
When the agent child restarts (for example after a crash is recovered), the
displayed and persisted cumulative values SHALL NOT regress to a lower count:
reports arriving from a freshly restarted child SHALL be merged monotonically
(never below the persisted value), and subsequent completed turns SHALL
continue accumulating on top of the preserved value. A crashed turn itself
SHALL NOT erase or distort the contributions of previously completed turns.

#### Scenario: crash does not roll back the accumulator

- **WHEN** a session has completed turns accumulating to a nonzero usage total
- **AND** the agent crashes (`crash` trigger) and the session recovers with further turns
- **THEN** the session head's cumulative usage never shows a value below the pre-crash total
- **AND** the value after recovery equals or exceeds the pre-crash total and grows with each completed turn

#### Scenario: reload keeps the preserved total

- **WHEN** the page is reloaded after a crash-and-recover sequence
- **THEN** the session head shows the preserved cumulative total, not the restarted child's fresh count

### Requirement: Model-switch receipts are operator-driven and uniform

A successful operator-initiated model switch SHALL append exactly one system
receipt entry to the conversation transcript naming the previous and the new
model, and this SHALL hold identically for every agent kind (Claude driver,
generic ACP agents, native kernel). Model information reported by the agent
child on its own initiative — including the startup default after a crash
recovery and any re-report on child restart — SHALL NOT produce a switch
receipt. A rejected switch keeps the existing typed rejection presentation and
SHALL NOT produce a success receipt.

#### Scenario: switch receipt appears for every agent kind

- **WHEN** the operator switches the model of a session, for each agent kind (claude, generic ACP, native)
- **THEN** exactly one system receipt naming the old and new model appears in the transcript
- **AND** the next turn runs under the chosen model

#### Scenario: child restart does not fake a switch receipt

- **WHEN** the agent child crashes and the session recovers
- **AND** subsequent turns run on the child's restart default model
- **THEN** no「模型已切换」receipt appears for the restart-implied model
- **AND** a later operator-initiated switch still produces exactly one receipt naming the actual previous and new model
