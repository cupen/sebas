## 1. Tokens and assets (`static/` only — no rebuild needed)

- [x] 1.1 Subset and add the four self-hosted font files under `static/fonts/`
      (Archivo 400/600, Archivo Narrow 600, Martian Mono 400, Latin subset,
      woff2); verify each loads by requesting its `/static/fonts/*` path and
      confirm the total added weight is under 150 KB
- [x] 1.2 Vendor `marked` and `highlight.js` into `static/vendor/` alongside
      `htmx.min.js`, with a `static/vendor/README.md` recording the pinned
      versions; verify each file is served from `/static/vendor/*`
- [x] 1.3 Declare the token block in `style.css` `:root` — the six core tokens
      and six status colors from design.md D2, plus `@font-face` rules, spacing
      and radius scales; verify by grepping the stylesheet for color literals
      outside the token block and finding none
- [x] 1.4 Add the light and dark scheme token overrides under
      `prefers-color-scheme`; verify both schemes' body and status colors meet
      4.5:1 (3:1 for large text) with a contrast checker

## 2. Status vocabulary in Rust

- [x] 2.1 Add a display-only status label and glyph to `SessionRow`, computed
      from `(MappingState, phase)` per the design.md D7 mapping table; verify a
      unit test covers all seven input rows including `Active` + empty phase
- [x] 2.2 Populate the new field at both construction sites in `routes.rs`;
      verify no template still reads the raw `phase` field by grepping
      `templates/` for `.phase` and finding no direct render

## 3. Shell and live updates

- [x] 3.1 Rebuild `base.html`'s shell against the tokens — drop the emoji nav
      glyphs, add the status ribbon slot and the skip-to-content link; verify
      every page still renders and the nav highlights the current page
- [x] 3.2 Add the single `EventSource('/events')` client in `base.html`, with a
      400 ms-debounced `htmx.ajax('GET','/sessions/partial')` on each `update`
      message; verify by starting a session and watching the board update
      without a manual reload
- [x] 3.3 Keep exactly one `showToast` in `base.html` and add the missing
      `.toast` / `.toast-container` rules; verify a toast is visibly styled by
      triggering an admin action and confirm the duplicate copy in
      `admin_update.html` is gone

## 4. Session list

Superseded by `add-project-workbench`, which demotes the flat board out of
primary navigation: the pulse lane and the "lane board" signature are dropped
from this change. What remains is the styling the list needs either way.

- [x] 4.1 Restyle `sessions_partial.html` against the tokens — status gutter,
      glyph, condensed status word, middle-truncated mono identity with the full
      value in `title`, age, actions; verify `/` and `/sessions` both render it
      from the one partial
- [x] 4.2 Style the focused-session gutter in `--signal` and confirm it is the
      only orange on the page; verify by grepping the stylesheet for `--signal`
      usage and checking each hit is focus marker, primary button, or focus ring
- [x] 4.3 Replace the five stat cards with the one-line ribbon on `/`, and
      render uptime as a human-readable duration; verify the raw second count
      appears nowhere in the response body

## 5. Remaining pages

- [ ] 5.1 Restyle `session_detail.html` against the tokens and add the missing
      `.timeline-*`, `.message-*`, and `.status-dot` rules it and the markdown
      renderer depend on; verify a session with a markdown body and a code block
      renders formatted with a styled copy button
      **BLOCKED — the restyle is done, the markdown half is not.** Two premises
      of this task turned out to be false. (a) The `.timeline-*`/`.message-*`/
      `.status-dot` rules had no markup left to style once 7.1 deleted
      `agent.html` and `agent_timeline.html`; they were removed as dead CSS
      instead of added — as was `.timeline-scroll`, which the class audit had
      been treating as live only because an orphaned `scrollToBottom()` in
      `base.html` still named it. Both are gone. (b) The card body is agent-controlled and `marked`
      passes raw HTML through, so rendering it as markdown is an HTML-injection
      decision, not a styling one — it is left as escaped text pending a
      decision. Consequence: `renderMarkdown()` selects `.md-content`, which no
      template now emits, so `marked.min.js` + `highlight.min.js` (157 KB) load
      on every page and do nothing. See the note under section 8.
- [x] 5.2 Restyle `settings.html`, `gateway.html`, and `about.html`, including
      the `.page-body` rule all three reference; verify each page renders with
      no unstyled block
- [x] 5.3 Rewrite the focus-scope copy so it states that focus changes only what
      the console displays and never message routing, and rewrite both empty
      states to name the action that fills them; verify the strings appear on
      `/sessions` and on `/` with no sessions present

## 6. Admin cluster

- [x] 6.1 Convert all five `admin_*.html` to `{% extends "base.html" %}` with an
      admin nav block, deleting the five duplicated HTML shells; verify
      navigating `/sessions` → `/admin/status` leaves the nav and brand chrome
      unchanged in position
- [x] 6.2 Add the missing `.action-grid`, `.form-card`, `.btn-warning`, and
      `.badge-running` rules the admin templates reference; verify
      `/admin/status`, `/admin/events`, `/admin/update`, and `/admin/services`
      each render with no unstyled block
- [x] 6.3 Render admin mutation controls in an explicit unavailable state naming
      the missing `SEBAS_CONTROL_SECRET` when no adapter is configured; verify
      by loading `/admin/update` without the secret and confirming the buttons
      explain themselves rather than failing on click

## 7. Remove dead surface

- [x] 7.1 Remove the `/agent` nav item from `base.html` and delete the
      unregistered `agent.html` and `agent_timeline.html`; verify every nav link
      in the rendered shell returns a page and none 404
- [x] 7.2 Remove the four non-functional filter tabs from `sessions.html` and
      their rules; verify no control on the page is inert
- [x] 7.3 File a follow-up issue for the agent chat page recording the four
      routes it needs; verify the issue exists and references this change

## 8. Quality floor and verification

- [x] 8.1 Add `:focus-visible` rings on every interactive element and accessible
      names on every glyph-only control; verify by tabbing the full board and
      confirming each stop shows a ring distinct from its hover state
- [x] 8.2 Add the responsive rules down to 375 px — collapsing nav and reflowing
      session rows; verify at 375 px that no page scrolls horizontally and every
      session's view, focus, and end-session controls are reachable
- [x] 8.3 Audit every class referenced across `templates/` against the
      stylesheet; verify the used-minus-defined difference is empty, treating
      the ~40 currently-unbacked classes as the starting worklist
- [ ] 8.4 Load the console with outbound network blocked; verify all pages render
      with the intended typography and that markdown bodies and code blocks
      display formatted rather than raw
      **First half done and now enforced by a test; second half blocked on 5.1.**
      `tests/rendered_markup_test.rs::no_page_references_an_external_asset`
      asserts no page references an `http(s)://` or `//` asset, and every
      `/static` reference resolves to a file on disk (3 woff2, 57 KB total).
      Markdown bodies still render raw — see 5.1.
- [x] 8.5 Render the board in grayscale; verify each status stays identifiable
      from its label and glyph alone
- [x] 8.6 Update `webui/tests/session_endpoints_test.rs` for the rewritten
      markup, asserting on data attributes rather than class names; verify
      `cargo test -p webui` passes
- [x] 8.7 Run `cargo build` and `cargo clippy -p webui`; verify both are clean
      and every template edit is reflected in the compiled binary
