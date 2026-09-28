## ADDED Requirements

### Requirement: 命令提交回执可见

Submitting an advertised slash command (including the built-in `/compact`)
SHALL produce a visible receipt in the submitting client: either a
transcript entry for the command invocation, or a notification-layer
acknowledgement naming the command. Silence — no transcript entry, no
notification, no state change the operator can see beyond `last active` —
SHALL NOT satisfy this requirement. The receipt SHALL distinguish accepted
commands from rejected ones: a rejected command keeps the existing inline
rejection, an accepted one gets a positive receipt.

#### Scenario: /compact 提交有回执

- **WHEN** the operator submits `/compact` to a started session
- **THEN** a visible receipt (transcript entry or notification naming the
  command) appears without a page reload

#### Scenario: 接受与拒绝可区分

- **WHEN** an accepted command and a rejected command are each submitted
- **THEN** the accepted one produces a positive receipt while the rejected
  one produces the existing typed inline rejection
