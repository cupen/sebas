## 1. feishu-reactions spec rewrite

- [ ] 1.1 Retarget reaction application to the IM service frontend (vocabulary, ack, phase machine, swap, target=card, terminal states).
- [ ] 1.2 Remove the in-flight back-pressure (⏳) and shared-debounce-pump cadence requirements, with Reason/Migration.

## 2. im-service spec

- [ ] 2.1 Add the "IM 前端渲染会话级 reaction" requirement (phase-driven, snapshot realign after resync).
- [ ] 2.2 Strengthen the ownership boundary in "IM 交互状态机随迁" (core SHALL NOT render IM reactions/cards).

## 3. Code/comment reconciliation (no behavior change)

- [ ] 3.1 Fix the self-contradictory comment in `sebas-dispatch/src/engine/acp_events.rs:125` (WORKING→DONE reaction) and align `engine/mod.rs` FSM comment + `card_state.rs` terminal-emission note with reality.
- [ ] 3.2 Note in code that core `Out::React` is a dead path on the feishu-less pump (or mark for removal).

## 4. Validation

- [ ] 4.1 `openspec validate --changes` passes; archive checklist run.
