# QA Report — Cluster A: sebas WebUI Management Surfaces (Black-box GUI)

- **Target**: http://127.0.0.1:9877/ (sebas WebUI sandbox, auth disabled, login-free)
- **Run 1**: 2026-09-29 00:19–00:34 CST (truncated by environment incident E-A1)
- **Run 2 (rerun)**: 2026-09-29 00:40–01:05 CST (completed all remaining points)
- **Tester**: QA subagent (automated browser)
- **Sandbox root**: `C:/Users/cupen/AppData/Local/Temp/sebas-qa/`

## Method & tooling disclosure

1. The session's mandated browser tooling (`browser-use` plugin via `mcp__node_repl__js`)
   **hard-fails in subagent context**: `setupBrowserRuntime` throws
   `Error: Browser is not available in subagent` (from the node-repl host's
   `assertAvailable`, before any browser API exists). Reported per the control-browser
   skill's own rule; **fallback**: Playwright 1.54 headless Chromium (repo's
   `tests/testsuite-webui` node_modules, read-only reuse — no repo files modified).
2. Discipline identical to the skills: real user-level interactions only (click/fill on
   snapshot-located elements); **no side-effect JS injection, no URL construction to bypass
   UI, no force-click, no refresh-to-escape**; every verdict cross-validated with a
   structured DOM read (ARIA snapshot and/or role/text locator reads) **plus a viewed
   screenshot**. Console/page-error listeners registered read-only at start of every script.
3. Shadow-DOM caveat: the app renders inside (nested) shadow roots. The **ARIA snapshot
   fold does not descend into `sebas-settings-modal`**, but Playwright role/text locators
   pierce it, so structured reads there were done via count/inner-text locator queries;
   raw `document.querySelector` cannot see any of it (light DOM is an empty shell).
   The Add-project dialog and all rail content ARE visible to the ARIA fold.
4. Read-only `curl` GETs used only to corroborate health/incident. Verdicts from GUI only.

## Environment incident E-A1 — RESOLVED (by orchestrator restart)

Run 1 was truncated at ~00:28 CST when the sandbox stopped externally: port 9877 empty,
`core-channel.sock` cleanly removed, logs end without error (deliberate/graceful stop,
not a crash). **Resolved by the orchestrator at ~00:37 CST** (detached core + router
restart — orchestrator's job, never the agent's). Run 2 started from equivalent state
(session/project registry empty — Run 1 had registered nothing).

State note: during Run 2 projects `work` and `downloads` were registered and then
**removed again via the GUI** (both removals verified), and agent `qa-a-guiagent` was
created, edited, and **deleted via the GUI** (non-resurrection verified). Final state ==
initial state. The 6 seeded agents were never deleted or edited (only `qa-a-*`).

---

## Test points

### A1 — First load & shell — **PASS**

- **Steps**: open `/`, wait for SPA settle, ARIA snapshot + screenshot.
- **Expected**: dashboard renders, no blank panes/broken chrome; nav areas inventoriable.
- **Observed**: complete shell — project rail (logo + “你忠诚的 AI 伙伴”， PROJECTS header
  with `+` Add-project button, empty-state “尚未注册项目”， Settings entry at rail bottom);
  workbench (context strip “未选择项目 未聚焦任何会话”， centered empty-state, lower
  composer pane with hint, resize separators). Console errors 0, page errors 0.
- **Verdict**: **PASS**. Evidence: `a1_shell.png` (viewed), `a1_shell.aria.txt`.

### A2 — Project management — **PASS** (D4 known-pending reproduced; rename unsupported; 1 cosmetic defect)

Completed in Run 2 (locator rebuilt from fresh snapshot facts: the modal is a native
`<dialog>`; rail button class `add-btn` vs modal button `has-label`):
1. Register `work`: tree select → path textbox auto-fills `…/sebas-qa/work` → disabled
   button enables → click → **toast 「项目「work」已注册。」** (top-center banner with
   close button, persists >3 s) → rail row `work local` with actions `…` and `+ new
   session` buttons and “该项目暂无会话”； main panel auto-focuses (header `work local`).
2. Second project `downloads`: same flow, toast shown, rail lists both, auto-focus.
3. **Focus switching**: clicking a project row switches the main context strip
   (`work local` ↔ `downloads local`), DOM + screenshots verified. Fresh page load starts
   unfocused (“未选择项目”) — project focus is not persisted across reload (observation).
4. **Rename: NOT supported** — the row actions menu contains exactly one item:
   「移除项目」. No rename affordance anywhere in the GUI.
5. **Remove + D4**: confirm dialog (“Remove project / 移除项目 **work**？ /
   此操作只解除注册，可重新添加。” with red 移除/取消 — honest copy) → after confirm:
   toast 「项目「work」已移除注册，可重新添加。」; **rail drops `work` but the main panel
   header keeps showing `work local 未聚焦任何会话`** — the known pending item **D4 is
   reproduced**. Reproduced again removing `downloads`: with **zero** projects left, rail
   shows “尚未注册项目” while main still shows `downloads local`.
- **Verdict**: **PASS** for all supported flows; **KNOWN-PENDING (D4) confirmed live**;
  rename unsupported (recorded); **DEFECT D-A2 (P3)** below.
- **D-A2 (P3, cosmetic)**: in the Add-project modal, the `or` separator abuts the bottom
  border of the fixed-height directory list; with the list scrolled to the last row the
  clipped row and the `or` label crowd the border. Repro: open Add project, scroll dir
  list to bottom. Actual: no spacing between scroll viewport edge and separator.
  Expected: clear spacing. Evidence: `a2_add_dialog.png`, `a3_oob_state.png`.
- Evidence: `a2f_toast_400ms.png`, `a2f_settled.png`, `a2p3_two_projects.png`,
  `a2p3_actions_menu.png`, `a2p4_focus_work.png`, `a2p5_after_remove_click.png`,
  `a2p5_d4_settled.png`, `a2p6_all_removed.png` (all viewed).

### A3 — Out-of-bounds registration — **PASS**

- **Steps**: Add project → type `C:/Windows` into Project path → observe; re-check with a
  valid in-root path; Cancel.
- **Expected**: rejected or not offered.
- **Observed**: out-of-root dirs are never offered in the tree (picker rooted at sandbox
  workspace root). Manual OOB path → **inline red validation
  「路径在 workspace root 之外——只能注册工作区内的目录」** and the Add project button
  stays `disabled` (native disabled attribute confirmed via locator read); a valid path
  re-enables it. A 400 Bad Request appears in the console from the validation call —
  expected backend rejection, handled gracefully by the UI.
- **Verdict**: **PASS** (explicit, well-worded rejection). Evidence: `a3_oob_state.png`
  (viewed), `a3_oob_state.aria.txt`.

### A4 — Agent directory (Settings → Agents) — **PASS** (1 defect, friction recorded)

- **List**: native builtin (chip `builtin · sebas`, note 「内置 sebas 内核（不可删除）」， no
  edit/delete buttons) + the 6 seeded ACP agents (claude, emptybot, errbot, slowbot,
  thinker, tooler), all `reachable`, each with ✎ edit and 🗑 delete. Intro text explains:
  config.toml `[acp.agents.*]` is only the first-start seed source; Settings-managed store
  wins; changes apply without restart. Seeded agents were only listed, never edited/deleted.
- **ADD `qa-a-guiagent`**: 「＋ New agent」 opens a form with **Agent id / Display name
  (optional) / Shape（claude（二进制路径））/ Binary path**. Filled id+display name
  `qa-a-guiagent`, binary path `D:/workbench/repos-ai/sebas/target/debug/fake-claude.exe`
  → Save → success banner 「已创建 qa-a-guiagent（免重启，创建会话下拉即可选）」； row
  appears in alphabetical position.
  - **Friction recorded (M-A4)**: the GUI form offers **no sessions_dir, no work_dir, no
    scenario-args (`--scenario …`) fields** — a GUI-only user cannot fully replicate the
    config-file agent definitions that the seeded agents use. Adapted to the fields that
    exist; the scenario-args requirement of the original brief is not achievable via GUI.
  - **DEFECT D-A4 (P3, data fidelity)**: the Display name filled at creation was not kept
    visible anywhere afterwards — the Edit form opened with Display name **empty** (it had
    been filled as `qa-a-guiagent`), and the created row showed no display-name chip
    (a chip only appeared after the display name was changed to a value ≠ id). Either the
    create silently dropped the value or the edit form fails to prefill; both variants are
    user-visible fidelity bugs. Repro: create agent with display name → click ✎ → Display
    name field is empty. Expected: stored value prefilled. Evidence: `a4_edit_form.png`.
- **EDIT**: pencil opens “Edit agent qa-a-guiagent” (Agent id read-only; Launch definition
  combobox defaults to 「不改（保留当前启动定义）」). Changed display name →
  「qa-a-guiagent-edited」 → banner 「已更新 qa-a-guiagent」； row shows the new chip.
- **DELETE**: trash opens confirm dialog with honest copy (“Delete agent
  **qa-a-guiagent**? 已建会话不受影响（继续到自然结束）；引用它的项目默认 agent 会被清除。
  之后在创建会话下拉中立即可见。”) → Delete → banner 「已删除 qa-a-guiagent（已建会话继续
  到自然结束）」； row gone immediately. Minor: confirm button is English "Delete" in an
  otherwise-Chinese dialog.
- **Reload persistence**: after full page reload the agent is **not resurrected** and the
  6 seeded agents + native remain intact. Zero console errors throughout.
- **Verdict**: **PASS** with **D-A4 (P3)** and friction **M-A4** recorded.
- Evidence: `a4_agents_list.png`, `a4_new_agent_form.png`, `a4_new_agent_filled.png`,
  `a4_after_save_settled.png`, `a4_edit_form.png`, `a4_after_edit.png`,
  `a4_delete_confirm.png`, `a4_after_delete_confirmed.png`, `a4_after_reload.png` (all viewed).

### A5 — Settings → Models/Providers — **PASS** (M7 gap not reproduced: relationship explained)

- **Observed**: header 「管理模型与 provider。预设派生值跟随应用代码；API key 由你自己保管。」;
  buttons `+ New (preset)` / `+ New (custom)`; honest default-model state **"no default
  set"**; store list empty ("No providers configured."); below it the config provider
  **anthropic** appears with chip **「config.toml · 只读」** and base URL
  `https://api.anthropic.com` — the API key (sk-sandbox-dummy) is **not** displayed.
  An explanatory paragraph states the two classes — store rows (editable, stored in the
  state DB) vs config rows (config.toml `[provider.*]` seeds, read-only here, edit the
  config file and restart) — and that session model pickers derive from session-reported
  configOptions/aliases, same source as this provider set.
- **Verdict**: **PASS** — the config-vs-store relationship **is explained inline** (the
  known gap M7 as briefed is **not reproduced** in this build); key secrecy respected.
- Evidence: `a5_models.png`, `a5_models_full.png` (viewed).

### A6 — Settings → Skills — **PASS**

- **Observed**: intro explains browse/remove + Sync projection and that the page never
  creates/edits skills (CLI/git/npx then Refresh). Empty store shows **"0 skills in
  store"** and an honest non-broken empty state: "The skill store is empty. Add skills
  with `sebas skills add`, git or npx — then Refresh." **Refresh** runs without error
  (state unchanged; no explicit success ack — minor note). `Sync` button present (not
  exercised: projection mutates backend skill dirs; out of Cluster A scope).
- **Verdict**: **PASS**. Evidence: `a6_skills.png`, `a6_skills_after_refresh.png` (viewed).

### A7 — Settings → About — **PASS** (build-time/git rows absent = known pending; 1 defect)

- **Observed**: ~1–4 s skeleton shimmer, then two sections:
  - **INSTANCE**: Workspace root `C:\Users\cupen\AppData\Local\Temp\sebas-qa` (with copy
    button ⧉; value wraps to a second line under the label — minor); Default agent kind
    `claude (default kind for new sessions)`.
  - **BUILD**: **Version chip `0.1.0`**; **Uptime `21m`**; **Rust toolchain
    `rustc 1.98.1 (48a229cea 2026-09-01)`** — a real value, **M9 answered: not 未知**；
    **Router listen `127.0.0.1:18887`** (matches the router's actual bind from its log);
    **Providers `1` (router 侧计数，含 debug provider)**.
  - **Build-time row and git branch@hash row: ABSENT** — the known pending change
    `add-about-build-info` is **confirmed still pending** in this build.
  - **DEFECT D-A7 (P3)**: the Rust toolchain row ends with a dangling grey **「要求 ≥」**
    whose version bound is missing (text node literally ends at "≥") — the required
    minimum toolchain is either truncated or not supplied. Repro: open Settings → About,
    read the Rust toolchain row. Actual: `rustc 1.98.1 (48a229cea 2026-09-01) 要求 ≥`.
    Expected: `… 要求 ≥ <version>`.
- **Verdict**: **PASS** for what exists; KNOWN-PENDING (build rows) confirmed;
  **D-A7 (P3)** recorded.
- Evidence: `a7_about.png` (skeleton), `a7_about_4s.png` (loaded, viewed),
  `a7_about_9s.png`.

### A8 — Notifications after key operations — **PASS** (M6 gap not reproduced)

- **Observed feedback for every key operation**:
  | Operation | Feedback |
  |---|---|
  | Project register | toast 「项目「work」已注册。」 (persistent, close button) |
  | Project remove | toast 「项目「work」已移除注册，可重新添加。」 |
  | Out-of-bounds path | inline red validation text |
  | Agent create | inline green banner 「已创建 qa-a-guiagent（免重启，…）」 |
  | Agent edit | inline banner 「已更新 qa-a-guiagent」 |
  | Agent delete | inline banner 「已删除 qa-a-guiagent（已建会话继续到自然结束）」 |
  | Skills Refresh | no error; **no explicit ack** (state simply unchanged) — only silent op found |
- **Observation**: toasts are clearly visible in screenshots but are **absent from the
  ARIA snapshot fold** (caveat: the fold also skips the whole settings modal, so this may
  be a tooling artifact rather than an app a11y defect — flagged for manual a11y check,
  not counted as a defect).
- **Verdict**: **PASS** — the briefed M6 "silent operations" gap did not reproduce on any
  tested operation (Skills Refresh's missing ack is the sole candidate, minor).
- Evidence: `a2f_toast_400ms.png`, `a2p5_d4_settled.png`, `a4_after_save_settled.png`,
  `a4_after_edit.png`, `a4_after_delete_confirmed.png` (all viewed).

### A9 — Persistence across reload — **PASS**

- **Observed**: final full page reload — shell renders cleanly (no blank panes/broken
  chrome), zero console/page errors; the deleted agent stays deleted and the 6 seeded
  agents persist (verified in A4's reload step); projects list empty as expected (both
  test projects were removed through the GUI). Agent-side changes survive reload;
  project focus resets to 未选择项目 (acceptable, recorded at A2).
- **Verdict**: **PASS**. Evidence: `a4_after_reload.png`, `a9_final_reload.png` (viewed),
  `a9_final_reload.aria.txt`.

### A10 — Visual quality pass — **PARTIAL PASS** (visited surfaces: shell, Add-project
dialog, remove dialog, delete dialog, agents/models/skills/about/appearance/services/
env-vars panels)

- **Clean**: shell layout, alignment and contrast; all dialogs; agent rows (long binary
  path fits its input without truncation); Models panel; Skills empty state; Appearance
  theme cards; About two-section table.
- **DEFECT D-A10 (P3, layout)**: Settings → Env Vars table — the **USED FOR column is
  squeezed to one word per line** (e.g. “Router config file path（未设置，默认
  ~/.sebas/config.toml）” wraps across ~9 lines) while the VALUE column stretches wide;
  the long `SEBAS_STATE_DIR` value reaches the panel's right edge (appears clipped).
  Repro: Settings → Env Vars. Actual: unbalanced column widths + edge-touching value.
  Expected: balanced columns, padded values. Evidence: `a10_env_vars.png`.
- **Minor notes (not counted as defects)**: Services unavailable-state box wraps its
  sentence awkwardly around the inline `sebas run` code chip (text order hard to scan at
  a glance); About Workspace-root value wraps to a second line; D-A2's `or` crowding and
  D-A7's dangling 「要求 ≥」 are filed under their own points.
- Evidence: `a10_appearance.png`, `a10_services.png`, `a10_env_vars.png` (all viewed).

---

## Evidence index (all in `C:/Users/cupen/AppData/Local/Temp/sebas-qa/qa-shots/`)

| File | Point | Viewed |
|---|---|---|
| `a1_shell.png` + `.aria.txt` | A1 | yes |
| `a2_add_dialog.png`, `a2_work_selected.png` (+.aria.txt) | A2 (run 1) | yes |
| `a2f_toast_400ms.png`, `a2f_settled.png` (+.aria.txt) | A2 register+toast | yes |
| `a2p3_two_projects.png`, `a2p3_actions_menu.png` (+.aria.txt) | A2 list/menu | yes |
| `a2p4_focus_work.png` (+.aria.txt), `a2p4_focus_downloads.png` | A2 switching | yes |
| `a2p5_after_remove_click.png`, `a2p5_d4_700ms.png`, `a2p5_d4_settled.png` (+.aria.txt) | A2/D4 | yes |
| `a2p6_all_removed.png` (+.aria.txt) | A2/D4 second data point | saved (DOM verified; superseded by d4_settled) |
| `a3_oob_state.png`, `a3_oob_state_full.png` (+.aria.txt) | A3 | yes |
| `a4_agents_list.png`, `a4_new_agent_form.png`, `a4_new_agent_filled.png`, `a4_after_save_toast_window.png`, `a4_after_save_settled.png` | A4 add | yes |
| `a4_edit_form.png`, `a4_edit_filled.png`, `a4_after_edit.png` | A4 edit | yes |
| `a4_delete_confirm.png`, `a4_after_delete_confirmed.png`, `a4_after_reload.png` | A4 delete+reload | yes |
| `a5_models.png`, `a5_models_full.png` | A5 | yes |
| `a6_skills.png`, `a6_skills_after_refresh.png` | A6 | yes |
| `a7_about.png`, `a7_about_full.png`, `a7_about_4s.png`, `a7_about_9s.png` | A7 | yes |
| `a10_appearance.png`, `a10_services.png`, `a10_env_vars.png` | A10 | yes |
| `a9_final_reload.png` (+.aria.txt) | A9 | yes |
| `report-A.md` | this report | — |

All screenshots were captured by the harness directly into `qa-shots/`; no console-error
lines beyond A3's expected 400 were observed in any run (every script prints its own
console/page-error tail; all empty except that one).

## Summary table

| ID | Scope | Verdict | Notes |
|---|---|---|---|
| A1 | First load & shell | **PASS** | 0 console/page errors |
| A2 | Project management | **PASS** | rename unsupported; **D4 known-pending reproduced (2×)**; D-A2 P3 |
| A3 | Out-of-bounds registration | **PASS** | explicit red validation + disabled submit |
| A4 | Agent directory CRUD | **PASS** | D-A4 P3 (display-name fidelity); M-A4 friction (no sessions_dir/work_dir/args fields) |
| A5 | Models/Providers | **PASS** | M7 not reproduced — config vs store explained inline; key hidden |
| A6 | Skills empty state + Refresh | **PASS** | honest empty state; Refresh clean (no ack) |
| A7 | About BUILD section | **PASS** | build-time/git rows absent (add-about-build-info pending confirmed); M9 answered (real rustc value); D-A7 P3 |
| A8 | Notification feedback | **PASS** | M6 not reproduced — all key ops give visible feedback |
| A9 | Persistence after reload | **PASS** | agents persisted/deleted stick; shell clean; 0 errors |
| A10 | Visual quality | **PARTIAL PASS** | D-A10 P3 (Env Vars column squeeze) + minor notes |

## Counts

- **Test points**: 10 — PASS 9 · PARTIAL PASS 1 (A10) · FAIL 0 · BLOCKED 0
- **New defects**: 4, all **P3** — D-A2 (modal `or` separator crowding), D-A4 (agent
  display-name fidelity), D-A7 (About Rust row dangling 「要求 ≥」), D-A10 (Env Vars
  column squeeze / edge-clipped value)
- **Known-pending confirmed live**: D4 (removed project's stale view — reproduced twice),
  add-about-build-info (build-time + git rows absent from About BUILD)
- **Briefed gaps checked**: M6 not reproduced (all key ops give feedback) · M7 not
  reproduced (relationship explained inline) · M9 answered (real toolchain value) ·
  M-A4 recorded (GUI form lacks sessions_dir/work_dir/scenario-args fields)
- **Environment**: E-A1 resolved-by-orchestrator-restart; sandbox left in initial state
  (projects/agents registry clean, seeded 6 untouched)

## Top findings

1. **D4 reproduced (known pending)**: after removing a project the rail drops it but the
   main workbench keeps showing the removed project's context (`work local`), even with
   zero projects left — exact repro + screenshots at A2.
2. **M-A4 (GUI coverage gap)**: the New-agent form offers only id/display/shape/binary
   path — sessions_dir, work_dir and scenario args are unreachable from the GUI, so a
   GUI-only user cannot rebuild what the seeded config agents define.
3. **D-A4 (P3)**: agent Display name filled at creation is lost or not prefilled in the
   Edit form (empty field, no name chip until changed to a non-id value).
4. **D-A7 (P3)**: About's Rust toolchain row shows a real `rustc 1.98.1` build but the
   「要求 ≥」 minimum-version bound is missing/dangling.
5. **D-A10 (P3)**: Env Vars table's USED FOR column collapses to one word per line while
   the state-dir value touches the right edge.
6. **Positives**: out-of-bounds project paths get a clear red validation + disabled
   submit; config-vs-store provider relationship is explained inline and the API key is
   never rendered; agent delete confirm copy is exceptionally honest; every key operation
   produces visible feedback; zero unexpected console errors across the entire run.
