# Cluster B1 — Session lifecycle & streaming (black-box GUI acceptance)

- Target: sebas WebUI sandbox http://127.0.0.1:9877/ (auth disabled), Chinese UI.
- Method: Playwright headless Chromium (reusing `tests/testsuite-webui/node_modules`), real user-level
  interactions only (click / keyboard / wheel). Every point cross-validated with ARIA snapshot (DOM)
  + a VIEWED screenshot. Console/pageerror/requestfailed listeners registered per run (read-only).
- Setup performed before formal testing: registered project `qa-b`
  (`C:\Users\cupen\AppData\Local\Temp\sebas-qa\work\qa-b`) via the Add-project dialog tree picker.
  Two sessions created and renamed via GUI: `qa-b-s1`, `qa-b-s2`. No sebas process was restarted or reconfigured.
- Run date: 2026-09-29 (times in screenshots are local). Run duration ≈ 35 min.
- Console errors across all runs: **none**, except the 400s documented under D-B11 (they are caused by the defect itself).

---

## B1 — Session creation

**Steps:** Sidebar "Add project" → dialog (dir tree / Project path / Execution node) → select `work` → `qa-b` → Add project.
Then project row "+ / New session in qa-b" → dialog (Agent=claude, Permission mode=Ask) → 「创建会话」.
Session actions (⋯) menu → 重命名 → rename to `qa-b-s1`.

**Expected:** session appears in sidebar list; workbench opens with composer.

**Observed:**
- Toasts 「项目「qa-b」已注册。」 and 「已在「qa-b」创建新会话。」; project row shows session count; session appears nested under the project with a live dot; workbench shows session header (🔒 claude / local / 逐次询问 / last active), empty-transient state 「还没有对话」 and a composer (placeholder 「开始对话…」, Permission-mode combobox, model chip, Send disabled while empty).
- **Only one creation flow exists**: the dialog has Agent + Permission mode but **no initial-prompt field** — creation is always "create empty, then message" (the first composer send is effectively the initial prompt). A "create with initial prompt" variant was not found anywhere.
- Sessions are auto-named with a UUID (`c8bab339-…192b29db`); rename to a human name works via Session actions → 重命名 (「留空则回退到首条消息预览」placeholder documents the fallback naming too).
- Status note on the dialog: 「尚未配置 provider 模型——仍可创建会话…」 honestly reflects the sandbox; creation proceeds with the agent default model.

**Verdict: PASS** (single-flow; initial-prompt-with-creation flow = not present, recorded as design observation, not scored MISSING since the chart said "test both if both exist" — one exists and works).

**Defect found in this area → D-B11** (path textbox mangles backslashes; see below; it blocks the *typed path* variant of project registration but not the tree-picker variant).

**Evidence:** `b1_add_dialog.png`, `b1_qab_selected.png`, `b1_registered.png`, `b1_new_session_ui.png`, `b1_session_created.png`, `b1_session_actions.png`, `b1_rename_ui.png`, `b1_renamed.png`.

---

## B2 — Basic turn

**Steps:** Type `hello qa-b` in composer → click Send. Capture at +400 ms (mid-turn) and after completion.

**Expected:** user entry appears; status transitions visible; assistant reply = echo of the message; order/timestamps sane.

**Observed:**
- User entry (`你` avatar, right-aligned, timestamp 01:14:15) appears immediately; assistant reply visible by +400 ms with the Send button flipped to a red **stop button (「停止回复」)** — i.e. a clear in-flight indicator — and flipped back to Send [disabled] when done. No textual "Queued/Running/Done" status label exists anywhere; the button flip + 「last active Ns ago」 + sidebar dot are the status surface (fast stub makes any label hard to catch).
- **Reply content is `hello world`, not an echo of `hello qa-b`.** This is the fake stub's canned greeting (per AGENTS.md the fake-claude answers "hello world"); the task brief's "echo of your last message" did not reproduce on this build. UI-side everything else is correct. B3 re-checks with a second message.
- Composer placeholder changes to 「Ask for follow-up changes…」 while/after a turn exists; model chip flips `default` → `Fake` once the turn resolved.
- A **「~1 NEW SINCE YOU LAST VIEWED | mark all seen」 divider** appeared above the reply even though the user was actively viewing the conversation → D-B12 below.

**Verdict: PASS** (with content-expectation caveat: echo behavior not present in stub; recorded, not a UI defect) + **D-B12 (P3)**.

**Evidence:** `b2_before_send.png` (composer filled, Send enabled), `b2_midturn.png` (reply visible, stop button red, "Fake" chip), `b2_timeout.png` (settled state — poll text never matched a literal "Done" because no such label exists; the settled DOM is in `b2_midturn.aria.txt`).

---

## B3 — Conversation continuity

**Steps:** Send second message `second qa-b message` in the same session.

**Expected:** reply reflects the NEW message; transcript order correct; first turn not duplicated.

**Observed:** transcript order strictly `hello qa-b → hello world → second qa-b message → hello world`, timestamps 01:14:15 / 01:15:52, no duplication, no reordering after reload. Reply content again canned `hello world` — the stub does not echo, so *semantic* continuity (context carry-over) is not demonstrable with this stub; UI continuity (single history, correct pairing, session kept) is correct.

**Verdict: PASS** (UI continuity; content-echo expectation not achievable against this stub — noted for the record).

**Evidence:** `b3_midturn.png`, `b3_final.png`. (Reload between B2 and B3 also showed the D-B12 divider had cleared only via reload.)

---

## B4 — Incremental streaming (`drip`) — key liveness point

**Steps:** In `qa-b-s2` send `drip`; sample the assistant entry every ~100–150 ms inside the same script run; screenshots at chunk boundaries.

**Expected:** partial text appears incrementally before the turn ends; final text complete, not duplicated/corrupted.

**Observed (sampled timeline, turn = drip0 drip1 drip2 @ ~400 ms cadence):**
```
6ms   run=true  "drip0"
405ms run=true  "drip0 drip1"
790ms run=true  "drip0 drip1 drip2"
959ms run=false (ended)
```
Chunk 1 was visible at 6 ms — long before turn end; chunks accumulated incrementally with no duplication or corruption; final DOM paragraph = `drip0 drip1 drip2` (verified in `b4_final.aria.txt`); mid-stream screenshot shows partial `drip0 drip1` rendered live with the red stop button up.

**Verdict: PASS.**

**Evidence:** `b4_stream_1.png` (mid-stream, partial "drip0 drip1" on screen), `b4_stream_0/2.png`, `b4_final.png`.

---

## B5 — `stream` (5 frames with pauses)

**Steps:** Send `stream`; sample every ~80 ms until end.

**Expected:** Running persists between frames; no premature Done; final content complete.

**Observed:** full text `chunk0 chunk1 chunk2 chunk3 chunk4` was already present at the first sample (9 ms) — the 5 frames render faster than the sampling floor, so frame-by-frame arrival was not visually resolvable. The turn **stayed in the running state (stop button up) for ~840 ms after the content was complete** — consistent with the stub's "250 ms pause before result" — and there was **no premature Done** (running flag never dropped and re-rose; it dropped exactly once at the end). Final content complete, single paragraph, correct order.

**Verdict: PASS.**

**Evidence:** `b5_stream_0.png`, `b5_stream_1.png`, `b5_final.png`.

---

## B6 — Long transcript (`flood`)

**Steps:** Send `flood` in `qa-b-s1` (1200 chunks back-to-back). Probe: ARIA snapshot latency as a responsiveness proxy, DOM token count (virtualization check), wheel-up/wheel-down scroll test. (A second probe run re-sent exactly `flood` to sample during a live ingest; a first attempt with `flood two` correctly fell through to the plain reply — the stub triggers on exact content `flood`.)

**Expected:** page responsive during ingest; long content rendered (note virtualization if any); scrolling works; no freeze.

**Observed:**
- All **1200 tokens `f0…f1199` are present in the DOM** — no virtualization, no small-summary truncation. After the second flood: 2400 tokens, still fine.
- Ingest completes fast: the first flood's turn finished inside the 5 s stop-button wait window; the second flood rendered all 1200 new chunks within ~2 s.
- **Responsiveness:** baseline ARIA-snapshot latency 6–10 ms; during the live-ingest probe one sample took **1183 ms** (a main-thread stall while the 1200-chunk paragraph rendered), all other samples stayed ≤ ~20 ms. UI never locked up; no console errors.
- Scrolling: wheel-up scrolls to the very top (oldest turn `hello qa-b` visible), wheel-down returns to the newest; rendering stays clean at both ends.
- Long paragraph wraps correctly (many lines, no horizontal overflow, no clipped text).

**Verdict: PASS** with a **P3 performance note (D-B13)**: one ~1.2 s main-thread stall during 1200-chunk ingest.

**Evidence:** `b6_diag.png` (flood turn completed), `b6_final2.png` (mid-transcript view: wrapping across ~15 lines), `b6_scroll_up.png` (top of transcript), `b6_scroll_down.png`, `b6_final.png`.

---

## B7 — Stop / cancel mid-turn

**Steps:** Start a `drip` turn in `qa-b-s2`; as soon as 「停止回复」 appears, click it; sample button label + transcript every ~100 ms; then observe the settled state. (Executed twice; second run for precise timing.)

**Expected:** immediate visual feedback on click; turn ends; final transcript wording recorded. (Known pending item D3 claims a pending-cancel presentation was NOT yet implemented — verify what a user sees today.)

**Observed timeline (second run, click at t=0):**
```
47ms   button still 「停止回复」           (no instantaneous change in the first sample)
162ms  button 「停止中…」                  (pending state appears)
…      stays 「停止中…」 ~0.9 s
1152ms button back to Send; turn ended
```
- Final transcript: partial assistant content **preserved** (`drip0` only), followed by a red error card:
  **「! 错误 · 回合被停止（操作者中断了本次回复）」** with its own timestamp (turn 01:22:23 → stop card 01:22:24). Composer returns to Send [disabled].
- **D3 status: the pending-cancel presentation IS present in this build** — the button visibly transitions 停止回复 → 停止中… → Send. If D3 meant something richer (e.g. a distinct "cancel queued turn" path), that was not observable here; what a user sees today is a labelled stopping phase plus the explicit stopped-wording card. Recorded as **KNOWN-PENDING-D3 → observed implemented (discrepancy with the claim)**.
- Stop latency total ≈ 1.1 s from click to ended turn (fake stub; a real agent may differ).

**Verdict: PASS** (+ KNOWN-PENDING-D3 reconciliation above).

**Evidence:** `b7_before_stop.png`, `b7_after_click.png`, `b7_stopping_mid.png` + `b7_final2.png` (settled state: two stopped turns, each partial `drip0` + red 「回合被停止」 card), `b7_settled.png`, ARIA: `b7_right_after_click.aria.txt`, `b7_final2.aria.txt`.

---

## B8 — Reload persistence

**Steps:** With both sessions holding completed turns (s1: 5 turns incl. 2×flood; s2: 5 turns incl. 2 stopped turns), s2 focused → `page.reload()` (the test action itself).

**Expected:** sessions list, per-session transcripts, focused session restore sanely; no broken panes.

**Observed:** after reload the sidebar lists project `qa-b` (count 2) with `qa-b-s2` and `qa-b-s1`; the **focused session (qa-b-s2) is restored** (header `qa-b-s2 🔒 claude local 逐次询问 last active 4s ago`); transcript intact to the last entry including both stop cards; composer, Permission combobox and model chip all present; layout unbroken; no console errors.

**Verdict: PASS.**

**Evidence:** `b8_after_reload.png`, ARIA `b8_after_reload.aria.txt`.

---

## B9 — Session switching

**Steps:** Click `qa-b-s1` → measure time until its transcript markers appear; then click `qa-b-s2` → same. Check for cross-session content bleed in both directions.

**Observed:**
- Switch → s1: transcript visible after **196 ms**; own content only (`hello qa-b…`, 2400 flood tokens ending `f1199`); **zero s2 content** (`chunk0…`/「回合被停止」 absent).
- Switch → s2: visible after **394 ms**; own content only (drip turn `drip0 drip1 drip2`, stream turn, 2 stop cards); **zero s1 content** (`f0 f1…`/`hello qa-b` absent).
- Header title, agent, permission mode all follow the selected session; sidebar selection highlight follows; scroll rests at the newest entry after each switch.

**Verdict: PASS.**

**Evidence:** `b9_s1.png`, `b9_s2.png`, ARIA `b9_s2.aria.txt`.

---

## B10 — Visual quality on visited surfaces

**Observed:**
- **Long-message wrapping:** the 1200-token flood paragraph wraps cleanly across lines at the pane width; no overflow, no clipped glyphs (`b6_final2.png`, `b9_s1.png`).
- **Composer during a running turn:** textbox remains present/editable, placeholder becomes 「Ask for follow-up changes…」, Send is replaced by the red stop square; Permission combobox and model chip stay put (`b2_midturn.png`). After stop/cancel it returns to Send [disabled] with the text cleared.
- **Entry spacing / alignment:** consistent rhythm user-right / assistant-left; timestamps center-aligned per entry; avatar column consistent (`b3_final.png`, `b5_final.png`); stop/error cards visually distinct (red tint) with their own timestamps (`b7_final2.png`).
- **Minor observation:** the composer model chip flips `default` → `Fake` once a turn resolves — it reflects the resolved model name; cosmetic, recorded not scored.

**Verdict: PASS.**

**Evidence:** `b2_midturn.png`, `b3_final.png`, `b5_final.png`, `b6_final2.png`, `b7_final2.png`, `b9_s1.png`.

---

## Defects & notable findings

### D-B11 — Add-project "Project path" textbox silently strips backslashes (P2)
- **Repro:** Add project → click 「Project path」 → type (real keystrokes, and same via programmatic fill) `C:\Users\cupen\AppData\Local\Temp\sebas-qa\work\qa-b`.
- **Actual:** the field shows `C:UserscupenAppDataLocalTempsebas-qaworkqa-b` (every `\` dropped), red hint 「路径不存在或无法访问——请检查路径是否正确」, Add button stays disabled; **each keystroke fires a validation request that returns HTTP 400** (seen as console errors `Failed to load resource: 400`).
- **Expected:** backslashes preserved (standard Windows absolute path) or an explicit message that only forward-slash paths are accepted.
- **Impact:** on Windows, typed/pasted backslash paths can never register a project; workaround = the directory tree picker (which fills the field itself with forward slashes and works). Severity P2 because a primary-looking input is silently broken with a confusing error, but a working alternative exists.
- **Evidence:** `b1_path_filled.png` (fill), `b10_path_typed.png` (keyboard typing), console 400s (unique console errors of the whole run).

### D-B12 — "NEW since you last viewed" divider shows and persists while the user is watching (P3)
- **Repro:** stay focused on a session and receive any reply (e.g. plain turn, drip, stream).
- **Actual:** divider 「~1 NEW SINCE YOU LAST VIEWED | mark all seen」 renders above the fresh reply although the conversation is on screen and fully visible; it persists (observed across minutes and multiple turns, e.g. 「~2 NEW」 in s2) until the page is reloaded or 「mark all seen」 is clicked.
- **Expected:** auto-mark seen when the entry is visible in the focused view (divider only for genuinely unseen content).
- **Evidence:** `b2_midturn.png`, `b5_final.png`, `b7_final2.png` (persists), contrast `b3_final.png` (gone after reload).

### D-B13 — ~1.2 s main-thread stall during 1200-chunk ingest (P3, performance)
- **Repro:** send `flood` and sample UI responsiveness (ARIA-snapshot latency as proxy) during ingest.
- **Actual:** one sample took 1183 ms vs 6–10 ms baseline (main thread blocked while the huge paragraph rendered); no lasting freeze, no errors.
- **Evidence:** probe table in B6 above; `b6_final2.png`.

### Expectation mismatches (not UI defects, recorded for the chart)
1. **Echo behavior absent:** plain messages answer canned `hello world` (B2/B3); the brief's "echo of your last message" did not reproduce on this stub build. UI ordering/continuity unaffected.
2. **No initial-prompt-with-creation flow:** session creation is always dialog → empty session → first message (B1).
3. **No explicit Queued/Running/Done status text:** turn phase is conveyed by the Send↔stop button flip, 「last active」 and the sidebar dot (B2).

---

## Summary

| ID | Point | Verdict | Key evidence |
|---|---|---|---|
| B1 | Session creation (dialog → empty session → composer; rename) | PASS | b1_registered.png, b1_session_created.png |
| B2 | Basic turn (entry, in-flight stop button, settle) | PASS (+D-B12) | b2_before_send.png, b2_midturn.png |
| B3 | Conversation continuity (order, no dup) | PASS | b3_final.png |
| B4 | Incremental streaming (`drip`) | PASS | b4_stream_1.png |
| B5 | `stream` frames, no premature Done | PASS | b5_final.png |
| B6 | Long transcript (`flood`), scroll, responsiveness | PASS (+D-B13 P3) | b6_scroll_up.png, b6_final2.png |
| B7 | Stop/cancel (停止中… → 回合被停止 card; partial kept) | PASS (D3 claim: observed implemented) | b7_stopping_mid.png, b7_final2.png |
| B8 | Reload persistence (list + transcripts + focus) | PASS | b8_after_reload.png |
| B9 | Session switching (196/394 ms, no bleed) | PASS | b9_s1.png, b9_s2.png |
| B10 | Visual quality (wrap, composer, spacing) | PASS | b6_final2.png, b7_final2.png |

**Counts: 10 PASS / 0 DEFECT-blocked / 1 DEFECT D-B11 (P2) / 2 minor D-B12, D-B13 (P3) / 3 expectation mismatches recorded / KNOWN-PENDING-D3 reconciled (pending-cancel presentation present in this build).**

**Console/page errors:** none across all test points, except HTTP 400 validation requests caused by D-B11 itself. No request failures otherwise; app never went down; sandbox untouched beyond the two `qa-b` sessions.
