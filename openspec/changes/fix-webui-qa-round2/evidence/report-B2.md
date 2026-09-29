# Cluster B2 — Message shapes & session metadata (black-box GUI acceptance)

- Target: sebas WebUI sandbox http://127.0.0.1:9877/ (auth disabled, Chinese UI).
- Method: Playwright headless Chromium (repo `tests/testsuite-webui/node_modules`, read-only reuse), real
  user-level interactions only (click / fill / keyboard / wheel). Every point cross-validated with ARIA
  snapshot (DOM) + VIEWED screenshots. Console / pageerror / requestfailed listeners registered per run
  (read-only). No JS injection, no URL bypass, no force-click, no refresh-to-escape (`page.reload()` used
  only where the chart itself demands a reload, in B19).
- Environment prep (before formal testing): created `C:\Users\cupen\AppData\Local\Temp\sebas-qa\work\qa-b2-work`
  on disk; registered it as project `qa-b2-work` via the GUI Add-project tree picker; created all sessions
  via the GUI New-session dialog; renamed via Session actions → 重命名. No sebas process restarted or
  reconfigured; seeded agents untouched. One accidental duplicate thinker session (created by a failed
  automation run) was kept and renamed via GUI (`qa-b2-think`).
- Run date: 2026-09-29. Sessions created under `qa-b2-work`: explain recursion please (thinker, auto-titled),
  qa-b2-empty, qa-b2-err, qa-b2-slow, qa-b2-crash, qa-b2-x, qa-b2-y, /goal (claude, auto-titled from a slash
  message), qa-b2-renamed-title (claude), qa-b2-depth, qa-b2-think (spare, unused).
- **Console errors across all runs: none** (console + pageerror + requestfailed all empty in every script run).

---

## B11 — Thinking blocks (thinker)

**Steps:** New session on `thinker` → send 「explain recursion please」 → sample transcript at 300 ms
intervals; second turn 「explain closures now」; third turn with 30–50 ms rapid sampling to catch a partial
state. Clicked the first thinking toggle to test collapsibility.

**Expected:** thinking entries rendered as DISTINCT visual blocks (collapsible?), interleaved order
thinking→text→thinking→text preserved; turn Done; mid-turn screenshot showing partial thinking.

**Observed:**
- Each thinking entry renders as a distinct chip **「⚡ PROCESS thinking 1」** followed by its text paragraph;
  order in every turn is strictly chip → text → chip → text (「thought out loud」 / 「and the answer」). Turn
  settles with composer back to Send [disabled]. ARIA: `b2_11_turn1_done.aria.txt`, `b2_11_turn2_final.aria.txt`.
- **Collapsible:** clicking the chip toggles `[expanded]` and reveals a dashed-border content area (stub
  thinking content: the word 「thinking」) between chip and text — `b2_11_after_toggle1.png`. No signature
  string is rendered anywhere (collapsed or expanded).
- **Mid-turn partial thinking could NOT be captured:** the thinker stub emits all frames within one sampling
  interval (turn complete at first sample, 43–300 ms). Three attempts (250 ms, then 30–50 ms rapid sampling)
  never caught a partial state. Evidence limitation of the instant stub, not an app defect — incremental
  rendering itself was already proven in cluster B1 (drip/stream).
- Cosmetic observation: both chips in the same turn are labelled 「PROCESS thinking 1」 (no per-block index
  increment visible).

**Verdict: PASS** (interleaving + collapsibility verified; mid-turn partial not capturable against the
instant stub — recorded honestly).

**Evidence:** `b2_11_turn1_done.png` (viewed), `b2_11_after_toggle1.png` (viewed, expanded state),
`b2_11_turn3_final.png` (viewed, 3 turns). Console errors: none.

---

## B12 — Empty turn (emptybot)

**Steps:** New session `qa-b2-empty` on `emptybot` → send 「hello empty」 → sample to settlement.

**Expected:** no crash; honest presentation (notice/placeholder/explicit empty state); status settles.

**Observed:** user entry appears; the turn is represented by a gray **info card**: 「ⓘ 提示 —
**回合已结束且无输出**：本轮回合未产生任何可见输出（正文、thinking、工具、错误皆无）。」 with its own
timestamp. No crash, no console errors; composer returns to Send [disabled]; session remains in the sidebar
with normal green dot.

**Verdict: PASS** — presentation is explicit and honest.

**Evidence:** `b2_12_final.png` (viewed), `b2_12_final.aria.txt`. Console errors: none.

---

## B13 — Error turn (errbot) + post-error UX

**Steps:** New session `qa-b2-err` on `errbot` → send 「trigger error please」 → capture error rendering →
then send a second message 「recover now」 in the same session.

**Expected:** error rendered in transcript (not silent); session reaches a failed/error state; second
message behavior recorded.

**Observed:**
- Error rendered as a red card: 「**! 错误** — upstream error (fake): provider returned 500」 with timestamp,
  visible at first sample (357 ms). Not silent.
- **Failed-state surface:** no textual "failed" badge on header or sidebar row; the observable failure
  surface is (a) the red card in the transcript and (b) the sidebar dot turning **gray** (vs green for
  healthy sessions) — visible in `b2_13_error_shown.png`.
- **Post-error recovery:** the session stays selectable; a second message sends normally and errbot answers
  「hello world」; the sidebar dot returns to green. Full recovery, no lock-out, no stale error state.

**Verdict: PASS** (recovery verified; note: failure state has no text badge — dot color + card only,
recorded as design observation).

**Evidence:** `b2_13_error_shown.png` (viewed), `b2_13_recovery.png` (viewed), `b2_13_final1.aria.txt`.
Console errors: none.

---

## B14 — Slow turn (slowbot, 4 s silence)

**Steps:** New session `qa-b2-slow` on `slowbot` → send 「take your time」 → sample every 250 ms across the
silence window.

**Expected:** in-flight indicator during the 4 s silence, no premature error; content arrives; settles Done.

**Observed:** running indicator (red stop button 「停止回复」) present at every sample from 298 ms through
3803 ms; the user entry additionally shows an **「已收到」** (received) acknowledgment label while in-flight;
flipped to settled at 4067 ms; reply 「slow reply」 rendered (01:40:41). **Zero premature errors** across the
whole silence.

**Verdict: PASS.**

**Evidence:** `b2_14_mid_silence.png` (viewed — red stop button up, no content, no error),
`b2_14_final.aria.txt`. Console errors: none.

---

## B15 — Crash mid-turn (claude, message `crash`) — key regression point

**Steps:** New session `qa-b2-crash` on `claude` → send exactly `crash` → 120 s watch at 500 ms sampling;
then refocus tests across fresh app loads; finally a post-crash recovery send.

**Expected:** "boom" appears then process dies; session reaches a CLEAR terminal state quickly (previous
round saw ~600 s zombie Queued — fix claimed).

**Observed:**
- **Immediate presentation is fixed:** 「boom」 paragraph + red card 「! 错误 — agent process exited or hung
  (watchdog)」 both visible at first sample (t=0.6 s); composer settled (Send [disabled]) by **t=1.6 s**.
  No visible hang. Sidebar dot for the crashed session turns gray.
- **BUT the backend turn stays stalled-running for up to 600 s.** Evidence trail:
  - ~11 min after the crash, refocusing the session intermittently showed a **running-state with no active
    turn**: composer flipped to the red stop button 「停止回复」 with Send absent and an idle transcript
    (`b2_probe3.png`, captured once; the `b2_b15b` run failed for 30 s because no Send button existed —
    the composer was stuck in stop-state and rejected a new send).
  - Fresh-load focus polls (3 rounds, 100 ms sampling) at a later time showed no stop state — intermittent.
  - The transcript eventually shows the reconciliation card: 「**! 回合停滞** — **回合停滞被强制收尾**：超过
    600 秒无任何事件（最后事件 613 秒前），队列中 0 条待执行提交已解除卡死。」 at 01:52:49 (crash was
    01:41:26). After it, sends work again.
- **Post-crash recovery send:** accepted; the stub re-crashed (「boom」 + watchdog card again) — expected,
  the conversation history contains `crash` and the stub triggers on it; UI-wise the error surfaced and the
  turn reached terminal quickly again.
- The 「~2 NEW SINCE YOU LAST VIEWED | mark all seen」 divider appeared while the view was open when the
  error landed (see B16 for semantics).

**Verdict: DEFECT D-B215 (P2)** — the *immediate* terminal presentation is genuinely fixed (fast watchdog
card, no 600 s UI hang), but the crashed turn remains a **bounded backend zombie (≤600 s)**: during that
window a refocus can present a running-state (stop button) that **blocks sending in the session**, until
the stall watchdog force-closes it.

**Repro (D-B215):** session on `claude` → send `crash` → within the next ~10 min, leave and re-enter the
session (intermittent; retry if it does not appear) → composer shows 停止回复 with an idle transcript; Send
is absent; sending is blocked until 「回合停滞被强制收尾」 appears (~600 s after the crash).
Actual-vs-expected: expected a clear terminal state with the session immediately reusable; actual = clear
*visual* terminal but session reusability can be blocked up to 600 s.

**Evidence:** `b2_15_boom.png`, `b2_15_terminal.png` (viewed — terminal state + divider), `b2_probe3.png`
(viewed — idle transcript with stop button = zombie window), `b2_15_recovery.png` (viewed — full timeline:
crash card → stall card → recovery send → second crash card). Console errors: none.

---

## B16 — Unread badge semantics (regression-prone, fix claimed)

**Steps:** Sessions `qa-b2-x`, `qa-b2-y` (claude). (1) In X send `drip`, immediately switch to Y, let X
finish unfocused; screenshot sidebar; refocus X. (2) Repeated with Y↔X including 100 ms fine sampling on
refocus. (3) Actively viewing X, complete another `drip` turn; check no unread marking. (4) Re-check the
B15 crash divider and 「mark all seen」 affordance.

**Expected:** unfocused completion → badge on X's row; focusing X → divider marking the unread boundary,
which then clears; actively-viewing completion → NO unread marking.

**Observed:**
- **Unfocused completion → badge: WORKS.** X's row showed a blue count pill **「1」**
  (`b2_16_sidebar_after_x_done.png`); repeated for Y (`qa-b2-y 1 local` in ARIA, `b2_16b_y_badge.png`).
- **Focus clears the badge:** on refocusing X the 「1」 pill is gone (`b2_16_x_focused_divider.png`).
- **Boundary divider on refocus: NOT SHOWN.** 100 ms fine-sampling from the refocus click caught no
  「NEW SINCE YOU LAST VIEWED」 divider at any sample (all `divider=false` from t=0). The unread boundary
  line the chart expects is missing — badge-only UX. → **M-B216**.
- **Actively-viewing completion (successful turn) → NO unread marking: WORKS** (fix from B1's D-B12
  verified): full `drip` turn completed in-view with zero dividers at every sample
  (`b2_16_active_view_end.png`, polls all `divider=false`).
- **Divider semantics observed elsewhere:** an **error** completion in an open view DOES surface the
  「~N NEW SINCE YOU LAST VIEWED | mark all seen」 divider (B15 crash, `b2_15_terminal.png`) — an attention
  affordance; it clears on next focus (revisit showed no divider). A successful unfocused→refocus completion
  shows the badge only.
- 「mark all seen」 button observed visually next to the divider but its click was not exercisable (the
  divider had already cleared on refocus before any click).

**Verdict: PASS on badge semantics (focused & unfocused both correct) + MISSING M-B216 (P3): no
unread-boundary divider is shown when refocusing a session with unseen successful turns.**

**Evidence:** `b2_16_sidebar_after_x_done.png` (viewed — badge 「1」 on X while Y focused),
`b2_16_x_focused_divider.png` (viewed — badge cleared, no divider), `b2_16_active_view_end.png`,
`b2_16b_y_badge.png`, `b2_15_terminal.png` (viewed — divider + mark-all-seen affordance on error),
`b2_16_x_focused_divider.aria.txt`. Console errors: none.

---

## B17 — Auto-title

**Steps:** Created an UNTITLED session (UUID name, never renamed) on `claude` → message #1
「THE-UNIQUE-HAIKU-REQUEST autumn leaves falling」 → check title → message #2
「SECOND-TOTALLY-DIFFERENT-MESSAGE zebra quantum」 → check title again → manual rename via ⋯ → 重命名 →
「qa-b2-renamed-title」 → check header + sidebar.

**Expected:** title derives from FIRST message preview; msg #2 must NOT change it; manual rename reflected
in workbench header (no internal id prefix) and sidebar immediately.

**Observed:**
- After msg #1 the sidebar row (and header) retitled to the full first-message preview
  「THE-UNIQUE-HAIKU-REQUEST autumn leaves falling」 — truncated with 「…」 for space (sidebar:
  「THE-UNIQUE-HAIKU-…」, header: 「THE-UNIQUE-HAIKU-REQUEST autumn leaves f…」).
- After msg #2 the title **stayed** at msg #1's preview (`TITLE-STABLE: true`).
- Manual rename: header immediately shows 「qa-b2-renamed-title 🔒 claude local …」 — no id/chat_id prefix —
  and the sidebar row updates in the same tick. Rename toast 「会话已重命名为「qa-b2-renamed-title」。」 shown.
- Corroborating earlier evidence: the thinker session auto-titled to 「explain recursion please」 from its
  first message; a session whose first message is a slash command auto-titles to the command text
  (「/goal」) — cosmetic oddity, noted under B18.

**Verdict: PASS** (all three clauses).

**Evidence:** `b2_17_untitled_created.png`, `b2_17_after_msg1.png`, `b2_17_after_msg2.png` (viewed),
`b2_17_after_rename.png` (viewed). Console errors: none.

---

## B18 — Slash commands (claude: `goal` with argument hint, `compact`)

**Steps:** New claude session → type 「/」 in composer → inspect menu; type 「co」 to filter; ArrowDown +
Enter keyboard selection; submit `/compact` (Enter-completion + Send click, and an Enter-submitted
variant); type unknown 「/nonexistent」 → Enter → Send.

**Expected:** menu lists commands; `/compact` produces a visible receipt/entry (fix claimed); unknown
command → honest behavior (error/no-op with feedback).

**Observed:**
- **Menu: GOOD.** Typing 「/」 opens a listbox 「Session commands」 with 「/goal <condition>」 (argument hint
  rendered) and 「/compact」, first option auto-selected, live tooltip with description (「Track a goal
  across turns」 / 「Clear conversation context」) — `b2_18_slash_menu.png`.
- **Filtering: WORKS** — 「/co」 narrows to 「/compact」 [selected]. **Keyboard: WORKS** — ArrowDown moves the
  selection; **Enter completes the command into the composer** (does not submit); Send submits it.
- **`/compact` submission:** shows toast 「命令 /compact 已提交。」 — visible feedback exists (the claimed
  fix, at toast level). **BUT in the transcript there is NO receipt/entry for the command**: no user
  bubble, no receipt line, and the command's reply content **merged into the previous assistant paragraph**
  (「hello world」 became 「hello worldhello world」 under the *older* turn's timestamp). The transcript is
  misleading after a compact. → **D-B218**.
- **Unknown 「/nonexistent」: HONEST.** With no menu match, Enter produces an inline status line
  「该会话的 agent 不支持此命令：/nonexistent」 and the composer **retains** the text; Send with the invalid
  command is a no-op (no turn starts, no silent swallow, no crash).
- Path inconsistency noted: the same 「/…」 text submitted via Enter-with-no-menu-selection is routed as a
  COMMAND (toast 「命令 /compact 已提交。」, no transcript bubble), while the same text submitted via the
  Send *button* is sent as a plain message (user bubble 「/goal」, canned reply, and it becomes the
  auto-title). Confusing dispatch, folded into D-B218.

**Verdict: PASS on menu / filtering / keyboard / unknown-command honesty + DEFECT D-B218 (P3): no
in-transcript receipt for `/compact` and command output merges into a previous assistant paragraph;
slash-text dispatch differs by submission path.**

**Evidence:** `b2_18_slash_menu.png` (viewed), `b2_18_slash_filter.png`, `b2_18_keyboard_selection.png`,
`b2_18_unknown_clean.png`, `b2_18_unknown_final.aria.txt` (status line), `b2_18_compact_final.png`
(viewed — merged 「hello worldhello world」 paragraph + command toast), `b2_18_after_compact.aria.txt`.
Console errors: none.

---

## B19 — Multi-turn depth

**Steps:** New session `qa-b2-depth` (claude) → 5 sequential messages (depth msg ONE…FIVE), each waited to
settlement → ARIA snapshot → reload → refocus → ARIA snapshot → entry-by-entry comparison.

**Expected:** order/labels stay correct; no reordering/duplication; reload → identical transcript.

**Observed:** strict alternation 你+timestamp → claude+timestamp for all 5 turns (01:50:20 → 01:50:41),
entries array identical before/after reload (`IDENTICAL: true`), user message order ONE→FIVE preserved, no
duplication. Per-turn token usage is not displayed anywhere in the transcript UI (nothing incorrect
observable — model chip flips to 「Fake」 after turns).

**Verdict: PASS.**

**Evidence:** `b2_19_before_reload.png`, `b2_19_after_reload.png`, `b2_19_before_reload.aria.txt`,
`b2_19_after_reload.aria.txt`. Console errors: none.

---

## B20 — Visual quality pass on visited surfaces

**Observed (all from VIEWED screenshots):**
- **Thinking blocks:** chip 「⚡ PROCESS thinking 1」 + dashed-border expanded area; clean spacing; no
  overflow (`b2_11_after_toggle1.png`).
- **Error cards:** red-tinted card, (!) icon, 「错误」 label, timestamp; also the 「回合停滞」 stall card with
  a long single-line text that fits without clipping at the tested content width; the empty-turn info card
  is a distinct gray ⓘ style — good visual hierarchy between error / stall / info
  (`b2_13_error_shown.png`, `b2_15_recovery.png`, `b2_12_final.png`).
- **Long error text wrap:** the longest available error text (stall card, ~60 CJK chars) renders on one
  line within the card; no clip/overflow. Longer error strings are not producible with the seeded agents
  (agent definitions off-limits), so extreme-wrap is untested — recorded as not exercisable, not a defect.
- **Badge rendering:** blue count pill 「1」 sits between name and 「local」 tag, no row-height jump
  (`b2_16_sidebar_after_x_done.png`).
- **Title truncation:** sidebar truncates with 「…」 at ~16–18 chars; workbench header truncates with 「…」
  at full width; no mid-grapheme clipping (`b2_17_after_msg2.png`).
- **Toasts (P3 polish observation):** toasts persist unusually long (creation/rename toasts still visible
  ≥40 s after the action, stacking 2–3 high with manual ✕ close) — `b2_12_final.png`,
  `b2_13_error_shown.png`, `b2_16_sidebar_after_x_done.png`. Noisy but not blocking.
- **Sidebar state dot:** green for healthy, gray for errored/crashed — a useful at-a-glance state surface;
  returns to green after a healthy turn.

**Verdict: PASS** (no styling defects found; toast persistence recorded as P3 polish note, not counted as
its own defect).

---

## Summary

| ID | Point | Verdict | Defect / Note |
|---|---|---|---|
| B11 | Thinking blocks | **PASS** | stub too fast for mid-turn partial (evidence limit); both chips labeled 「1」 |
| B12 | Empty turn | **PASS** | explicit 「回合已结束且无输出」 info card |
| B13 | Error turn + recovery | **PASS** | red card + gray dot; full recovery on next message |
| B14 | Slow turn (4 s) | **PASS** | stop-button + 「已收到」 during silence; no premature error |
| B15 | Crash mid-turn | **DEFECT D-B215 (P2)** | immediate terminal fixed (~1.6 s) but backend zombie ≤600 s can block sends on refocus; stall card reconciles at 600 s |
| B16 | Unread badge semantics | **PASS + MISSING M-B216 (P3)** | badge & no-false-unread correct; boundary divider on refocus absent; error completions show divider in-view |
| B17 | Auto-title | **PASS** | first-message preview; stable on msg #2; rename immediate |
| B18 | Slash commands | **PASS + DEFECT D-B218 (P3)** | menu/filter/keyboard/unknown-honesty good; no in-transcript compact receipt; output merges into prior paragraph; dispatch differs by path |
| B19 | Multi-turn depth | **PASS** | 5 turns correct; reload-identical |
| B20 | Visual quality | **PASS** | clean cards/badges/truncation; toasts linger (P3 polish) |

**Counts:** 10 points → **7 PASS**, **2 DEFECT** (D-B215 P2, D-B218 P3), **1 MISSING** (M-B216 P3),
0 KNOWN-PENDING. **Console/page errors: 0 across all runs.**

### Defect index
- **D-B215 (P2, B15):** crash leaves backend turn stalled-running ≤600 s; refocus in the window shows a
  stop-state and blocks sends until 「回合停滞被强制收尾」 card lands. Repro + actual-vs-expected in the B15
  section.
- **D-B218 (P3, B18):** `/compact` gives toast-only feedback; no transcript receipt; reply content merges
  into a previous assistant paragraph; slash-text dispatch (command vs plain message) depends on
  Enter-vs-Send-button path.
- **M-B216 (P3, B16):** no 「NEW SINCE YOU LAST VIEWED」 boundary divider when refocusing a session with
  unseen successful turns (the badge clears silently instead).

### Known-pending reconciliation
- B15's claimed fix (fast clear terminal state instead of ~600 s zombie Queued): **confirmed for the
  immediate UI presentation** (terminal at ~1.6 s with an explicit watchdog card); the residual 600 s
  backend stall is now *bounded and explained* (stall card) but still user-visible → recorded as D-B215
  rather than KNOWN-PENDING.
- B16's claimed unread-badge fix: **confirmed** for both claimed behaviors (badge on unfocused completion;
  no false unread while actively viewing on successful turns — B1's D-B12 no longer reproduces for
  successful turns).
