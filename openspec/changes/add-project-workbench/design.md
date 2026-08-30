## Context

See `proposal.md` — Why. Constraints and existing groundwork:

- **Half the plumbing exists.** `RouterHandle::web_spawn(prompt, project_dir)`,
  `Out::WebSpawn { key, prompt, project_dir }`, `Mapping.project_dir`, and
  `SessionMap::set_project_dir` are all in place. `sebas-webui/src/routes.rs:201`
  passes `None` and no route ever collects a directory.
- **The core is the only session authority.** `add-core-session-channel` makes the
  WebUI a client of it: `trait SessionBackend` replaces the WebUI's direct
  `RouterHandle`, and both the detached and in-process paths drive real sessions.
  This change assumes that seam and does not re-litigate it.
- **`~/.sebas/state.json` is the core's file.** `sebas-router/src/state_store.rs`
  rewrites it whole, atomically (tmp + rename), on every mutation. A detached
  WebUI writing the same file would clobber the core.
- **Session origin is derivable, per-turn origin is not.** `SessionKey::web_key()`
  distinguishes workbench-created sessions. Nothing records which surface an
  individual turn came from.
- **Depends on `redesign-webui-console`** for the token system, vendored assets,
  SSE consumption, the status vocabulary, and the accessibility floor, and on
  `add-core-session-channel` for live session data and drivability.

## Goals / Non-Goals

**Goals:**

- The project is the unit; several projects' agents are legible at once.
- Catching up is the primary job the layout serves.
- Every control's promise matches what the running process can actually do.

**Non-Goals:**

- No new router state and no writes to the core's state file.
- No changes to the channel protocol itself (that is `add-core-session-channel`).
- No file browser, diff review, or terminal inside a project.

## Decisions

### D1: Concurrency comes from the channel, not from a mode flag

An earlier draft of this change gated the composer on `can_drive: bool` — true on
`run --webui`, false on the watchdog-spawned process whose outbound receiver was
discarded — because the constraint at the time was to add no backend wiring.

That is withdrawn. Two facts made it untenable: the disabled path is the one
operators actually run (the watchdog spawns the detached WebUI; the spec labels
`run --webui` legacy), and the detached process was not merely undrivable but
showed the session set from the previous core exit. A flag would have shipped a
workbench that is inert in its own default configuration.

`add-core-session-channel` removes the condition instead of labeling it. So here:
the composer is unconditionally live, driving several projects concurrently is
just several sessions open against one core, and the only unavailable state left
is "core not connected" — transient, caused, and recovered from automatically per
that change's degradation contract.

What survives from the earlier draft is its reason for existing: a control that
reports success while discarding the message is the worst failure mode for a tool
whose job is trust in unattended work. That is now satisfied by making the
control work rather than by disabling it.

### D2: Project registry is a WebUI-owned file

`~/.sebas/projects.json`, written by the WebUI only, holding `{path, added_at}`
per project with the path as identity.

Rejected: adding a `projects` field to `state.json`. The core rewrites that file
in full on every mutation; two writers means last-writer-wins data loss, and the
WebUI is explicitly not the owner of core state.

Also rejected: putting the registry behind the session channel. The registry is a
view preference — which projects this operator wants listed — not session state.
Sending it to the core would make the core responsible for a concept it has no use
for, and `Mapping.project_dir` already carries the part the core does need.

Consequence: projects registered in the workbench are invisible to Feishu. A
Feishu-originated session still gets attributed to a project when its
`project_dir` matches a registered path.

### D3: Layout — three zones, catch-up first

```
┌──────────────┬───────────────────────────────────────┬────────────┐
│ PROJECTS     │ sebas                     main  ●3    │ SESSION    │
│              │ ~/work/sebas                          │            │
│ sebas    2 ● │ ───────────────────────────────────── │ origin  W  │
│ beads    1   │  14:02  you                           │ claude     │
│ dotfiles     │  重构 webui 的状态色阶                 │ 12.4k tok  │
│              │                                       │ 8 turns    │
│ FROM FEISHU  │  14:02  claude                        │            │
│ oc_9f2a  1   │  ▸ read  style.css                    │ perms      │
│              │  ▸ edit  style.css   +42 −18          │  edit  ✓   │
│ ──────────── │                                       │  bash  ?   │
│ + Add project│ ══ 3 new · while you were away ══════ │            │
│              │                                       │ ───────    │
│ All sessions │  14:31  you    继续                   │ End session│
│              │ ───────────────────────────────────── │            │
│              │ [ message ]        claude-sonnet-4 ⏎  │            │
└──────────────┴───────────────────────────────────────┴────────────┘
```

Left rail: registered projects, then the origin-named grouping for sessions with
no project directory, then the demoted cross-project list. A project row carries
its live session count and a single dot when one of its sessions is waiting on
the operator — nothing else. Middle: the project header (path, git branch when
readable) over the turn stream. Right: the session panel — origin, model, token
meter, turn count, permission grants, end-session.

The structural direction follows the brief's dsh reference: project rail, turn
stream, composer with inline context. What the brief leaves free is the visual
identity, and that is not spent on the generic-AI-chat look — no left/right
bubble alternation, no circular avatars, no gradient, no 16 px pills. Turns are
flush-left blocks separated by whitespace and a timestamp, because the stream is
a record to be read, not a conversation to be re-enacted; only two participants
exist, so sides carry no information worth a whole layout axis.

### D4: Signature — the "while you were away" seam

One horizontal seam in the stream marks the boundary between turns already seen
and turns that arrived since, with the count and the span. When a session has
unseen turns, opening it positions the stream **at the seam** rather than at the
newest turn.

It earns the signature slot because it encodes the property that actually
distinguishes sebas from every other agent UI: you are not the only one driving,
and you are usually not watching. Turns arrive from Feishu while you are in a
meeting and from agents running unattended for minutes. The operator's real
first question on opening a project is not "what is the latest message" but
"what did I miss" — and the scroll-to-seam behaviour is what turns that from
decoration into the thing that saves the work.

It is also cheap and semantically correct: the boundary is per-browser state in
`localStorage`, never server-side, because "while **you** were away" is a
property of a viewer, not of a session.

Only one signature. The pulse lane from `redesign-webui-console` does not
survive into the workbench — see D7.

### D5: `--signal` is reassigned to "your move"

The console change fixed the rule that status colors describe the machine and
the single accent describes the operator; there it marked the focused session.
In the workbench the thing that is about the operator is **the turn waiting on
them**: a pending permission request, and the composer when it holds focus. So
`--signal` marks those and nothing else. The seam does *not* get orange — it is
a time boundary, not a demand — and takes its own desaturated `--seam` token so
the two never compete.

Rejected: a per-project accent hue. With several projects on screen the
temptation is to color-code them, but it multiplies into badge soup, collides
with the status ramp, and breaks down past about five projects. The project name
set in condensed caps in the header carries the same information for free.

### D6: Turn stream reuses the existing card element pipeline

Sessions already carry a rendered body as `CardElementView` (`markdown`,
`collapsible`, `hr`, `div`) built for the Feishu card. The stream renders those
same elements rather than inventing a second representation, so the workbench
and the Feishu card cannot drift. Tool calls arrive as `collapsible` and render
collapsed with their summary line visible. The channel's turn method carries these
elements with the monotonic position that change defines, so the client fetches
only what it has not seen.

Consequence: turn boundaries are approximate, since the card body is a flat
element list rather than a typed turn log. The seam therefore anchors to element
index, and the "3 new" count is counted in elements-since-seen presented as
turns. Recording a real turn log is router state, which this change excludes —
noted as a known imprecision rather than hidden.

### D7: What the console change should drop

The board-centric group of `redesign-webui-console` — the lane board and the
pulse lane — is superseded: the flat board is demoted out of primary navigation,
so a signature element built for it no longer has a surface worth carrying it.
That change should keep its tokens, vendored assets, SSE wiring, status
vocabulary, admin shell dedup, and accessibility floor, all of which this change
builds on, and drop the board group. Stated plainly: the pulse lane was my
proposal one round earlier and the new product direction retires it.

## Risks / Trade-offs

- **Three unarchived changes, two of them touching the `webui` route-surface
  requirement.** Two unarchived deltas on one requirement conflict at archive
  time. → Archive order is `redesign-webui-console` → `add-core-session-channel`
  → this change. This delta is written against the post-console text; if the
  console change is abandoned, re-derive it from the current main spec.
- **This change now depends on a backend change landing first.** The workbench is
  unbuildable-as-specified until `add-core-session-channel` supplies the seam. →
  Accepted deliberately: the alternative was a workbench that is inert in the
  configuration the watchdog actually spawns. Sequence the work, do not overlap
  it.
- **Element-index seams drift if the card body is rewritten in place.** Card
  updates mutate earlier elements, so a seam can land mid-turn. → Anchor to a
  stable element identity where one exists, and treat the count as approximate
  in the copy ("3 new") rather than claiming precision.
- **Reading git branch shells out per project render.** N projects means N
  subprocess calls on every page load. → Read `.git/HEAD` directly and cache per
  path with a short TTL; omit the field rather than block the render.
- **A registered path can disappear or become unreadable.** → Show the project
  with its path marked unreachable and keep its sessions listed; never fail the
  page.
- **`projects.json` is unknown to Feishu.** → Accepted per D2; attribution of
  Feishu-originated sessions works by matching `project_dir` against registered
  paths.

## Migration Plan

Additive; no existing data changes shape.

1. Registry module first, on top of the landed session channel.
2. Project routes and rail, reading existing sessions grouped by `project_dir`.
3. Turn stream over the channel's turn method, then the seam.
4. Pass `project_dir` into session creation so workbench-started sessions
   attribute.
5. Demote `/sessions` out of primary navigation last, once the rail replaces it.

Rollback: revert the commits and delete `~/.sebas/projects.json`. No core state
was written, so nothing else needs undoing.

## Open Questions

- Whether a project should be able to pin a default prompt or preset for new
  sessions. Additive, does not change the registry shape, the routes, or the
  task breakdown — decide once the workbench is in daily use.
