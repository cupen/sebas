# Cluster C — Permission Modes & Approval Cards (black-box GUI acceptance)

- App: http://127.0.0.1:9877/ (auth disabled), 2026-09-29, Playwright headless Chromium 1440x900, locale zh-CN.
- Method: web-gui-tester discipline — ARIA snapshot + viewed screenshot per state; real user-level clicks/typing only; no side-effect JS, no URL bypass, no force-click. Transient states captured in-cell (before → action → wait → after).
- Environment prep (before formal testing): none beyond what earlier clusters left running; orchestrator-owned sandbox untouched. Sessions created under project `qa-b`: 8 fresh sessions (all auto-named `perm`/`parallel` by first message — see C10 observations).
- Console-error proxy: read-only `console`/`pageerror`/`requestfailed` listeners were registered in every run. **Zero console errors, zero page errors, zero failed requests across all 10 test points.** No visible error manifestations (blank screens, broken resources) observed either.

---

## C1 — Mode menu inventory

**Steps:** Fresh session → click the composer's mode combobox (bottom-left of composer) → inventory entries → hover each entry → close via Escape → reopen → close by selecting.

**Expected:** exactly four entries (ask/edit/allow/auto), current mode visibly indicated, hover shows a description tooltip, menu opens/closes cleanly, no layout defects.

**Observed:**
- Menu is a listbox that opens **upward** above the trigger. Entries: `Ask`, `Edit`, `Allow`, `Auto` — **exactly four, no legacy fifth**.
- Current mode visibly indicated: checkmark + filled highlight on `✓ Ask` (selected entry).
- Hover: each option carries a `title` attribute with a Chinese description — Ask=`逐次询问`, Edit=`自动接受编辑`, Allow=`放行并留审计`, Auto=`自动执行（不门控，留审计）`. Hover highlights the row. **Caveat (honest):** these are native browser title tooltips; headless Chromium screenshots cannot render them, so the tooltip *rendering* could not be visually confirmed — the descriptions are verifiably present in the DOM and surface elsewhere (dialog caption, header chip).
- Closes cleanly: Escape closes it; selecting an entry closes it; reopen works repeatedly.
- No layout defects; the dropdown overlapping the composer while open is normal dropdown behavior.

**Verdict: PASS** (tooltip visual rendering unverifiable in headless — noted, not counted against)

**Evidence:** `c_1_menu_open.png`, `c_1_hover_ask.png`, `c_1_hover_auto.png`, `c_1_menu_closed_esc.png`, `c_1_menu_closed_select.png`

---

## C2 — Default posture

**Steps:** Open "New session in qa-b" dialog → read Permission mode → create → inspect header + composer.

**Expected:** fresh session starts in ask mode; record how the mode is displayed.

**Observed:**
- Dialog: `Permission mode` combobox defaults to **Ask**, with the description `逐次询问` printed below it.
- After creation: session header shows a chip with the mode's Chinese label (`逐次询问`), plus a lock icon before `claude` (lock icon appears in **all** modes — it is decoration, not an ask-mode indicator); composer combobox reads `Ask`.
- Mode is displayed in **three** places: dialog caption, header chip, composer combobox.

**Verdict: PASS**

**Evidence:** `c_2_dialog_default.png`, `c_2_fresh_session.png`

---

## C3 — Approval flow in ask mode (allow path)

**Steps:** Send exactly `perm` → wait for card → screenshot pending → click `Allow once` → capture decision feedback → wait for settle.

**Expected:** card with tool name/input + approve/deny affordances; after allow, tool_result success (「perm done」), post-loop text, turn Done. Known-pending M1 claims: result double-folded with NO 「已执行」 feedback — verify what a user sees today.

**Observed:**
- **Pending card** (bottom panel above composer): shield icon + tool title `Bash`, `session …` / `request claude:tc-1` ids, JSON input block `{"command": "rm -rf /"}`, buttons **Allow once** (primary blue) / **Allow for session** (green outline) / **Deny** (red outline), plus an escalate row (textbox "Why raise this once? (escalate)" + **Escalate** button, disabled until text). Transcript shows a spinner process entry `PROCESS Bash · rm -rf /`. Status banner: 「会话在等你的权限批复：提交将排队，直到你处理审批」 + sidebar 等待 badge.
- **After Allow once:** card disappears immediately; the process entry collapses to `✓ PROCESS ✓ Bash 2 [✓ 已执行]` — **decision feedback IS visible today (green 已执行 chip)**. Expanding the fold shows two nested flat rows: `Bash · rm -rf /` (request) and `✓ Bash` (result) — the double-fold structure from M1 remains.
- **M1 re-check:** partially improved. The 「已执行/已拒绝」 feedback claimed missing now exists on the fold header. **But the tool_result content (「perm done」) is unreachable**: clicking either nested row collapses the entire parent fold (verified twice, including a fast capture at +150 ms and bounding-box check proving the nested row resolves onto the collapsed parent). No path renders the result text.
- **Post-loop text: absent.** The turn ended right after the tool entry — no assistant text paragraph followed (expected per brief). Consistently absent in every single-tool `perm` turn across all modes (C4/C6 too), while the two-tool `parallel` turn DID render its post-loop text (「parallel tools finished」, C5).
- **Turn Done: no explicit completion marker anywhere** (no "Done" status text). The only settle signal is the stop button (停止回复) reverting to Send. Subsequent messages work fine, so the turn did settle server-side.

**Verdict: DEFECT D-C3 (P2)** — tool_result content not viewable (nested fold rows not expandable; clicking collapses parent) + post-loop text missing on single-tool approval turns. M1 status: **KNOWN-PENDING-M1 (partially improved — feedback chip present, result content still hidden)**.

**Repro (D-C3):** ask session → send `perm` → Allow once → expand `✓ PROCESS` fold → click `✓ Bash` (or `Bash · rm -rf /`) → whole fold collapses; result text 「perm done」 is never rendered anywhere.
**Actual vs expected:** expected result content viewable + post-loop text; actual neither.

**Evidence:** `c_3_card_pending.png`, `c_3_card_pending_full.png`, `c_3_after_allow.png`, `c_3_fold_open.png`, `c_3_nested_collapse.png`, `c_3_no_postloop.png`

---

## C4 — Deny path

**Steps:** Fresh ask session → send `perm` → click `Deny` → capture feedback → wait for settle.

**Expected:** is_error tool_result rendered; turn still settles; post-loop text behavior recorded.

**Observed:**
- After Deny: process entry shows `✗ PROCESS ✗ Bash 2 [✗ 已拒绝]` — red chip, clear feedback. Turn settles quickly (stop button gone ~250 ms later). Session remains usable.
- The is_error tool_result *content* is likewise not viewable (same fold defect as D-C3); only the 已拒绝 chip conveys the outcome. No error banner, no honest-failure message — the turn ends silently.
- No post-loop text either.

**Verdict: PASS with the D-C3 caveats** (deny decision works, feedback clear, turn settles; error content not viewable → tracked under D-C3)

**Evidence:** `c_4_card_pending.png`, `c_4_after_deny.png`, `c_4_final.png`

---

## C5 — Parallel approvals

**Steps:** Fresh ask session → send exactly `parallel` → poll 30 s for TWO simultaneous cards → decide card 1 Allow once → observe → decide card 2 Deny → settle.

**Expected:** TWO approval cards pending at once, independently addressable; allow → success result, deny → error result; then post-loop text, Done.

**Observed:**
- **Only ONE `Permission review` card ever renders.** While the Bash card (`echo first`, `claude:tc-par-1`) was pending, the second tool_use (Read `/tmp/fake-parallel.txt`) sat as a plain process entry `PROCESS Read · /tmp/fake-parallel.txt 2` with **no decision affordance**. Held 30 s + re-check 15 s — never two cards.
- After Allow once on card 1, the Read card (`claude:tc-par-2`) **replaced** it — a serialized queue, not simultaneous. Approvals are therefore not independently addressable in one view; "which card is which" is answerable only one-at-a-time (cards do carry distinct tool titles + request ids).
- **Final transcript lost the allowed tool entirely:** only `✗ PROCESS ✗ Read 4 [✗ 已拒绝]` remains — the allowed+executed Bash process entry and its result are nowhere (its in-flight entry had been hidden behind the pending card and never re-appeared). The user cannot see that the first tool ran or what it returned.
- Post-loop text 「parallel tools finished」 rendered, then turn settled.

**Verdict: DEFECT D-C5 (P2)** — two parts: (1) parallel approvals render serially (one card at a time), second request addressable only after the first is decided; (2) after decisions, the allowed tool's process entry disappears from the transcript — executed tool + result visually lost.

**Repro:** ask session → `parallel` → observe single card + bare Read entry → Allow once → Read card replaces Bash card → Deny → transcript shows only the denied Read entry.
**Actual vs expected:** expected 2 simultaneous cards, per-tool results, both retained; actual 1-at-a-time and allowed entry gone.

**Evidence:** `c_5_single_card.png`, `c_5_serial_second_card.png`, `c_5_final_missing_allowed.png`

---

## C6 — Mode semantics (core question)

**Steps:** For each mode, create a FRESH session with that mode in the dialog, send exactly `perm`, record card vs auto-run. Ask-mode data from C3/C4.

**Expected (app's own design):** ask always asks; auto never asks; allow/edit — record actual.

**Observed:**

| mode | header chip | composer | card for Bash `rm -rf /`? | outcome |
|---|---|---|---|---|
| Ask 逐次询问 | 逐次询问 | Ask | **YES** | waits for decision (C3/C4) |
| Edit 自动接受编辑 | 自动接受编辑 | Edit | **YES** | card shown; Allow once → ✓ 已执行 |
| Allow 放行 | 放行 | Allow | no | auto-ran → ✓ 已执行 |
| Auto 自动执行 | 自动执行 | Auto | no | auto-ran → ✓ 已执行 |

- ask=always asks, auto=never asks: **as designed**. Edit gates a non-edit tool (Bash) — **sensible** (auto-accepts edits only; untestable further with this stub since it only emits Bash/Read).
- **Allow and Auto are observationally identical** for this flow: no card, immediate execution, identical `✓ 已执行` presentation. The descriptions promise a distinction (放行并留审计 "allow, keep audit" vs 自动执行（不门控，留审计） "auto-execute, no gating, keep audit") but nothing in the GUI differs — both kept the audit entry, neither gated.

**Verdict: ask/edit/auto PASS; DEFECT/MISSING recorded as M-C6 (P3): Allow vs Auto distinction has no observable GUI difference.** (Recorded as MISSING rather than DEFECT: the distinction may exist server-side for tool classes this stub cannot emit; with the available surface it is unobservable.)

**Evidence:** `c_6_edit_card.png` (Edit asks), `c_6_auto_final.png`, `c_6_allow_final.png` (identical no-card outcome), `c_6_auto_fresh.png`, `c_6_allow_fresh.png`, `c_6_edit_fresh.png`, `c_6_edit_final.png`

---

## C7 — Mode switch contract + persistence

**Steps:** In an existing session, switch mode via composer menu (Edit → Auto) → look for contract entry → reload page → re-focus session → check header/composer → check a *different* session's mode.

**Expected:** a `permission_mode_result` contract entry in the transcript; mode persists per-session across reload; per-session not global.

**Observed:**
- Switch immediately updates header chip (自动接受编辑 → 自动执行) and composer combobox (Edit → Auto), **and appends an info-style contract entry to the transcript**: `i 权限模式 / 权限模式已切换：自动执行` with timestamp — the fix is present and reads clearly.
- Note: re-selecting the *same* mode also logs an entry (seen in C3's transcript: 「权限模式已切换：逐次询问」 after a no-op re-select) — minor transcript noise.
- After page reload + re-focus: header chip and composer still Auto — **mode persists per-session**.
- A different session created in Allow stayed 放行/Allow — **per-session, not global**.

**Verdict: PASS**

**Evidence:** `c_7_contract_entry.png`, `c_7_after_reload.png`, `c_7_other_session.png`

---

## C8 — Queued message during pending approval (D1 regression)

**Steps:** Send `perm` → while card pending, attempt to send a second message via (a) Send button, (b) typing + Enter → approve → verify reply text appears exactly once, queued message runs after, order sane.

**Expected:** second message queues; after approval the reply appears once (no doubled deltas), queued turn runs after.

**Observed:**
- (a) While pending, the Send button is **replaced by 停止回复 (stop)** — a mouse user has no visible send affordance. This is the observed design: queueing happens via **Enter**.
- (b) Enter queues the message into a **queue strip** between card and composer: 「待执行 · 1 | queued via enter | 等待你的审批 · 已等 0 秒」 with per-item controls ↑ (上移) / ↓ (下移) / × (移除). Status stays 「会话在等你的权限批复…」.
- After Allow once: turn 1 completes (`✓ 已执行`), then the queued message runs as its own turn → reply `hello world` appears **exactly once** (string count in final transcript = 1), queued message appears once, ordering sane (approval turn first, queued turn second).
- **Historical dup-text bug D1: not reproducible — passes.**

**Verdict: PASS** (design note: Send-button path blocked while pending; queueing is Enter-only + strip management)

**Evidence:** `c_8_typed_pending.png`, `c_8_queue_strip.png`, `c_8_final.png`

---

## C9 — Decision persistence after reload

**Steps:** Fresh page load (every script run is a fresh browser session = reload equivalent); click through all `perm` sessions; verify decision history retained, nothing reset to pending.

**Observed:** 6 sessions inspected, all retained post-reload:
- ask + allowed → `process ✓ Bash 2 ✓ 已执行` (retained)
- allow/auto/auto modes → `✓ 已执行` (retained)
- ask + denied → `process ✗ Bash 2 ✗ 已拒绝` (retained)
- No session reset to a pending card; no stale 「等待你的审批」 status; no phantom approval prompts.

**Verdict: PASS**

**Evidence:** `c_9_session_ask_allowed.png`, `c_9_session_denied.png`

---

## C10 — Visual quality pass

**Observed:**
- **Approval card styling: clean.** Shield icon, bold tool title, muted session/request ids, monospace JSON input block, three clearly differentiated buttons (blue/green-outline/red-outline), escalate row. `c_3_card_pending.png`.
- **Decision chips:** ✓ 已执行 green / ✗ 已拒绝 red — legible and color-coded. `c_3_after_allow.png`, `c_4_final.png`.
- **Mode menu alignment:** opens upward, left-aligned to trigger, selected row highlighted; fine. `c_1_menu_open.png`.
- **Tooltips:** only native `title` attributes; headless rendering unverifiable (see C1).
- **Error tool_result styling:** not assessable — content unreachable (D-C3).
- **Long tool input wrapping:** not exercisable — the stub emits only short fixed inputs (`rm -rf /`, `echo first`, `/tmp/fake-parallel.txt`); no long-input case available through the GUI. Honest limitation.
- **Minor P3 findings:**
  - The 「last active Ns ago」 label freezes (observed stuck at "2s ago" across 45 s of polling) until the next data event refreshes it — stale relative time.
  - Session-creation toast (`已在「qa-b」创建新会话。`) overlaps the session header area while visible (transient, dismissible — cosmetic).
  - Auto-naming collides: every `perm`/`parallel` session gets the literal first-message name, leaving 6 indistinguishable `perm` rows in the sidebar (ambiguity when picking sessions; out of Cluster C scope, noted for Cluster B follow-up).
  - Model chip in composer flips `default` → `Fake` after the first agent turn in a session (observed in every qa-b session; likely the stub reporting a model name — cosmetic/out of scope).

**Verdict: PASS with P3 notes** (no P1/P2 visual defects)

**Evidence:** `c_10_escalate_typed.png`, `c_10_escalate_done.png` (escalate affordance: disabled → enabled with reason → acts as approve-with-reason), plus artifacts cited above.

---

## Summary

| ID | Point | Verdict | Severity |
|---|---|---|---|
| C1 | Mode menu inventory (4 entries, indicator, descriptions, open/close) | PASS | — |
| C2 | Default posture (ask default, 3 display surfaces) | PASS | — |
| C3 | Approval flow allow path | **DEFECT D-C3** (result content unreachable; post-loop text missing) + KNOWN-PENDING-M1 partially improved | P2 |
| C4 | Deny path | PASS (via D-C3 caveats) | — |
| C5 | Parallel approvals | **DEFECT D-C5** (serial cards; allowed entry vanishes) | P2 |
| C6 | Mode semantics | ask/edit/auto PASS; **M-C6 MISSING** Allow≡Auto unobservable | P3 |
| C7 | Mode switch contract + per-session persistence | PASS | — |
| C8 | Queued message during pending (D1 regression) | PASS (Enter-queue design) | — |
| C9 | Decision persistence after reload | PASS | — |
| C10 | Visual quality | PASS (4 P3 notes) | P3 |

**Counts: PASS 7 · DEFECT 2 (D-C3 P2, D-C5 P2) · MISSING 1 (M-C6 P3) · KNOWN-PENDING 1 (M1 partially improved)**

Top findings:
1. **D-C3 (P2):** after any decision the tool_result content (「perm done」/error) can never be viewed — nested fold rows collapse the parent when clicked; single-tool approval turns also drop the stub's post-loop text.
2. **D-C5 (P2):** parallel tool_uses queue one card at a time (never two simultaneous), and after decisions the allowed tool's process entry disappears from the transcript entirely.
3. **M1 re-check:** the claimed missing 「已执行/已拒绝」 feedback now exists on fold headers; the double-fold hiding the result remains.
4. **M-C6 (P3):** Allow and Auto are behaviorally identical in the GUI for the testable tool set; the promised distinction is unobservable.
5. **C7/C8/C9 healthy:** mode-switch contract entry renders, mode persists per-session across reload, Enter-queued messages run once in order (D1 dup-text not reproducible), decisions survive reload.

Console/page errors: **0 across all points** (listener-collected).
