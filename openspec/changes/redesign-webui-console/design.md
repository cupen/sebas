## Context

See `proposal.md` — Why. Constraints that shape the approach:

- **No build step.** minijinja templates compiled into the binary via
  `include_str!`, `static/` served from disk by `tower-http`. Template edits
  need a rebuild; `static/` edits do not. This asymmetry decides where logic
  lives.
- **`/events` already emits what we need.** `sse.rs` streams a named `update`
  event whose JSON payload carries `session_id` and, for `SessionUpdated`,
  `status`. Nothing consumes it today (no `hx-ext="sse"`, no `sse-connect`, no
  `EventSource` anywhere).
- **Status is a two-field derivation.** `SessionRow.status` is
  `active`/`spawning`/`dormant` from `MappingState`; `SessionRow.phase` is
  `CardState.status_emoji`, whose values are literally Feishu reaction
  `emoji_type` tokens (`sebas-router/src/card_state.rs`: `Get`, `OnIt`, `DONE`,
  `CrossMark`). The console currently prints these raw.
- **Loopback-only, single operator.** Bound to `127.0.0.1:9797`. No
  multi-tenant, no theming, no i18n machinery. But per README the operator
  reaches it from a phone, so small-viewport is a real requirement, not a
  nicety.
- **Dead surface exists.** `agent.html` / `agent_timeline.html` are not
  registered in `init_templates_inner()` and no `/agent` route is mounted,
  yet `base.html` links to it.

## Goals / Non-Goals

**Goals:**

- One token set every rule derives from, so drift can't reappear.
- The board tells the operator, in one glance from across the desk, which
  sessions are alive and which need them.
- Live updates with the fewest moving parts that work.
- Zero network dependency at render time.

**Non-Goals:**

- No new source of truth in the Rust layer. Data the design needs is either
  derived client-side from `/events` or projected from what `SessionRow`
  already carries — a display-only field computed from existing state is fine,
  a new piece of tracked state is not.
- No CSS framework, preprocessor, or npm. Hand-authored CSS with custom
  properties.
- No animation beyond the two moments named under Signature.

## Decisions

### D1: Design direction — "instrument readout", not "dashboard"

The subject is a process supervisor: every session is a long-running child
process with a heartbeat, a phase history, and a supervisor watching it. The
console's job is the job `htop` and a systemd status readout do — tell you
what's alive, right now, without being read word by word. So the design is
built as an instrument panel: dense, tabular, quiet, with exactly one live
element.

Rejected: the current direction, a generic admin dashboard (light-grey ground,
white cards, blue accent, emoji nav glyphs, five centered big-number cards).
It's the answer any brief would get. Also explicitly rejected the three looks
that AI design defaults to — cream + high-contrast serif + terracotta; near-black
with one acid accent; broadsheet hairlines at zero radius. The third is the
near miss here, since a supervisor readout *is* dense and tabular; the
differentiators are a consistent non-zero 3 px radius, a filled status gutter
per row instead of hairline-ruled cells, and condensed grotesque column labels
rather than editorial serif.

### D2: Palette — status colors describe the machine, the accent describes you

Seven core tokens:

| token | value | role |
|---|---|---|
| `--ink` | `#14161A` | graphite, primary text |
| `--ink-soft` | `#5C6270` | secondary text, labels |
| `--panel` | `#E7E9EC` | instrument grey, page ground |
| `--surface` | `#FBFCFD` | raised rows and panels |
| `--rule` | `#CBD0D7` | decorative hairlines between content |
| `--edge` | `#7E8086` | boundary of an operable control |
| `--signal` | `#E04E08` | indicator orange, the single accent |

`--rule` and `--edge` are split because they answer to different rules. A
hairline dividing two table rows is decorative and exempt from any contrast
minimum — keeping it faint is what makes the console quiet. The border of a
text input is how you know the input is there, which WCAG 1.4.11 requires to
clear 3:1. `--rule` measures 1.27:1 on `--panel`; using it on an input, as a
single-border-token draft did, would have been a real defect.

The status ramp is a separate axis, cool for settled, warm only for trouble:

| status | value | glyph |
|---|---|---|
| Starting | `#6A46F7` violet | `◇` |
| Queued | `#566A8A` slate-blue | `▹` |
| Working | `#0D728F` instrument cyan | `▶` |
| Done | `#1A7758` green-teal | `✓` |
| Failed | `#C42B24` alarm red | `✕` |
| Dormant | `#646972` slate | `·` |

Every status color clears 4.5:1 against **both** `--surface` and `--panel`
(measured 4.51–4.64 on the stricter ground), because a status word appears on
raised rows and on the page ground alike and must not depend on which. An
earlier draft of this table chose these hues by eye and shipped six values that
failed the very floor this change's "Accessibility floor" requirement sets —
Dormant at 2.64:1, Starting at 3.72:1, Working at 3.95:1. They were re-solved
numerically against the darker ground. The lesson is recorded here rather than
quietly corrected: a palette in a design document is a claim, and this one was
false until it was measured.

`--signal` moved `#F2560B` → `#E04E08` for the same reason. It has three jobs
with three different floors — a fill behind text (needs 4.5:1 for the label), a
graphic like the focused-row gutter (needs 3:1), and the focus ring. `#F2560B`
gave only 2.82:1 on `--panel`. `#E04E08` is the balance point: 3.28:1 as a
graphic, with `--ink` on it at 4.54:1. Hence the derived rule that `--signal`
is **never body text** — fill, border and ring only — and text on a signal fill
is `--ink` in light and `--panel` in dark, never white (white on `#E04E08` is
3.99:1 and fails).

The load-bearing decision: `--signal` is **not** a brand color and is never a
status. The product has exactly one concept that is about the operator rather
than the machine — the focused session, which the `webui` spec is explicit is a
display pointer that never changes routing. So the accent is reserved for
*where your attention is pointed*: the focused-session gutter, primary buttons,
and focus rings. Status colors say what the machine is doing; the orange says
what you are looking at. Nothing else gets to be orange.

Alternative considered: accent = Feishu brand blue `#3370FF` (close to today's
`#4361EE`). Rejected — it makes the console read as a Feishu surface when it is
the local operator's own instrument, and it collides with the Working cyan.

The dark scheme is a token override under `prefers-color-scheme`, not a second
stylesheet: `--ink #E8EAEE`, `--ink-soft #9BA2B0`, `--panel #101217`,
`--surface #191C22`, `--rule #2E333C`, `--edge #6B6D73`, `--signal #FF7A3D`,
and status `#A992FF / #93A9CC / #3FB8DB / #4FC395 / #FF7268 / #9BA2B0`. All
dark values clear 6.39:1, so dark mode is the comfortable one; light mode set
the constraint.

### D3: Typography — signage for labels, mono for identity

Three self-hosted OFL families, three roles:

- **Archivo Narrow** — column headers, status words, section labels. Condensed
  industrial grotesque with signage lineage; the condensation is what gives the
  board its readout character.
- **Archivo** — body and UI text.
- **Martian Mono** — session keys, chat/thread ids, counters, durations.
  Tabular figures so live-updating numbers don't jitter. Restricted to ≤13 px
  and short strings; it is wide, and using it for prose would be a mistake.

Rejected: system-UI stack alone (`-apple-system, …`, today's choice) — the type
carries none of the personality and the same stack renders three different ways
across the operator's machines. Also rejected Inter and JetBrains Mono as the
dev-tool defaults, and any Google Fonts CDN link, which would violate the
offline requirement.

Ships as **three** woff2 files, not the four static instances the tasks
anticipated: all three families are now served as variable fonts, so one file
per family covers every weight the console uses via `font-weight: 100 900`.
Latin subset, 57 KB total against a 150 KB budget.

### D4: Layout — one board, a status ribbon, no stat cards

`/` and `/sessions` render nearly the same table today. Keep both routes (the
`webui` spec requires them) but give them one shared board partial, with `/`
showing recent sessions and `/sessions` the full list.

```
┌──────────┬────────────────────────────────────────────────────────────┐
│ sebas    │ SESSIONS                              up 3d 04h · 17 total │
│ ──────── │ ┌────────────────────────────────────────────────────────┐ │
│ Sessions │ │ 2 WORKING   1 QUEUED   13 DORMANT   1 FAILED           │ │ ← ribbon, one line
│ Gateway  │ └────────────────────────────────────────────────────────┘ │
│ Settings │                                                            │
│ About    │ ▌▶ WORKING   oc_9f2a…d1 · th 7c          now  ⋯           │ ← row
│ ──────── │ ▌▹ QUEUED    oc_31bb…8e                  2m   ⋯           │
│ FOCUSED  │ ▌· DORMANT   oc_77c1…2f                  4h   ⋯           │
│ oc_9f2a  │ ▌✓ DONE      oc_04ae…9b                  1d   ⋯           │
│ End      │                                                            │
└──────────┴────────────────────────────────────────────────────────────┘
```

The five centered stat cards collapse into one ribbon line. That is deliberate:
a big number with a small label is the template answer, and these five numbers
are a glance-check, not the content. The content is the sessions.

Each row is: a 3 px status gutter (`▌`), glyph, status word in condensed caps,
identity in mono, age, and an actions menu. The gutter turns `--signal` orange
on the focused session — the only orange on the page.

### D5: Signature — withdrawn (superseded by `add-project-workbench`)

This change originally carried a per-row pulse lane — 16 status-colored ticks
with the newest pulsing while a session worked — as its signature element.

That is withdrawn. `add-project-workbench` makes the project the organizing unit
and demotes this flat board out of primary navigation, so a signature built for
the board no longer has a surface worth carrying it, and the workbench has its
own ("while you were away" seam). Two signature elements across two adjacent
surfaces is one too many.

What remains from the direction: everything static except row transitions. No
gradients, no shadows beyond a 1 px hairline, 3 px radius throughout.

### D6: Live updates — plain `EventSource`, not the htmx SSE extension

`base.html` opens one `EventSource('/events')`. On each `update` message it
schedules a debounced `htmx.ajax('GET', '/sessions/partial')` to refresh the
board.

Chosen over vendoring `htmx-ext-sse` because the workbench will need the message
payload itself (to place its seam), so an `EventSource` is required regardless;
adding the extension would mean carrying both. ~20 lines against a vendored file
and an `hx-ext` attribute on every board.

### D7: Status derivation lives in Rust, not the template

Add a `status_label` (and glyph) to `SessionRow` computed from
`(MappingState, phase)`:

| input | label |
|---|---|
| `Spawning` | Starting |
| `Active` + `Get` | Queued |
| `Active` + `OnIt` | Working |
| `Active` + `DONE` | Done |
| `Active` + `CrossMark` | Failed |
| `Active` + empty | Queued |
| `Dormant` | Dormant |

Templates then render a label, never a token. Doing this in minijinja would
scatter a six-branch conditional across three templates — which is how `Get`
and `OnIt` reached the screen in the first place. It is a projection of state
that already exists, not new tracked state.

### D8: Admin templates extend `base.html`

Five `admin_*.html` files each duplicate the full HTML shell and sidebar, and
`admin_update.html` carries its own second copy of `showToast`. Convert all
five to `{% extends "base.html" %}` with an admin nav block, and keep one
`showToast` in the base. This is what lets the shell-consistency requirement
hold without hand-syncing five files forever.

### D9: Vendor assets, delete dead surface

`marked` and `highlight.js` move from cdnjs into `static/vendor/`, joining
`htmx.min.js`; the two font families ship as woff2 under `static/fonts/` with
`font-display: swap`. Pin and record versions in a `static/vendor/README.md`.

Remove the `/agent` nav item and the two unregistered templates. The page needs
`/agent`, `/api/agent/projects`, `/api/agent/{key}/message`, and
`/agent/{key}/timeline`, none of which exist — that is a feature, not a
restyle, and belongs in its own change. Leaving a 404 in the primary nav while
claiming a design pass would be the worse outcome. Preserve the templates on
the branch history so the future change can recover them.

Also remove the four decorative filter tabs on `/sessions`, which the source
itself labels "purely visual". A control that looks clickable and does nothing
is a lie in the interface; either it filters or it goes. Filtering is not in
this change's scope, so it goes.

## Risks / Trade-offs

- **A busy daemon could refetch the board too often.** → Debounce the board
  refetch to 400 ms so a burst of SSE messages collapses into one request.
- **Martian Mono is wide; long chat ids will overflow on a phone.** → Middle-
  truncate ids in the row (`oc_9f2a…d1`) with the full value in `title` and on
  the detail page.
- **Two woff2 families add page weight to a localhost tool.** → Subset to
  Latin, ship only the weights used (Archivo 400/600, Archivo Narrow 600,
  Martian Mono 400), `font-display: swap`. Expect well under 150 KB total.
- **Template edits need a `cargo build`.** A designer iterating on CSS alone is
  fine; anyone touching structure must rebuild. → Keep as much as possible in
  `static/style.css`, and note the rebuild step in the tasks.
- **Existing tests assert HTML fragments.** `sebas-webui/tests/session_endpoints_test.rs`
  may match on markup this change rewrites. → Audit and update assertions in the
  same commits; prefer asserting on data attributes over on classes so the next
  restyle doesn't break them.
- **Removing `/agent` may surprise anyone who expected that page.** → It has
  never been reachable; call it out in the change summary rather than in a
  silent deletion.

## Migration Plan

No data migration; the change is presentation plus one derived field.

1. Land tokens, fonts, and vendored assets first — `static/` only, no rebuild,
   independently revertable.
2. Add `status_label` to `SessionRow` with unit coverage of the mapping table.
3. Convert `base.html`, then the board partial, then the remaining pages, then
   the admin cluster.
4. Delete dead surface last, once nothing references it.

Rollback: revert the commits. `static/` is served from disk, so restoring the
previous `style.css` alone recovers the old look for anything not yet
restructured.

## Open Questions

- Whether the ribbon should show gateway reachability alongside session counts.
  Additive, and it does not change the token set, the board, or the task
  breakdown — decide when the ribbon is on screen.
