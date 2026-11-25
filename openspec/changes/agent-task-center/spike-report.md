# Spike report: beads as the task-center engine

> **Durable copy.** The original spike ran in `/tmp/bd-spike-1795352597/`
> (ephemeral; the directory is gone). This file is the change-local copy of
> its conclusions so `design.md` has a stable pointer. Measurements below are
> the spike's recorded results; the "Re-verified" notes were re-run while
> landing relay 3 (2026-11-24) on `bd 1.3.4 (7ef2fd05e)`.

## Question

Can sebas use beads (a repository-local issue tracker) as the storage and
collaboration engine for a project task center, behind a thin HTTP + WS
wrapper — without reimplementing claim, lease, journal, or comments?

## Findings (as recorded in `design.md` / `proposal.md`)

| Fact | Result |
|---|---|
| `bd events tail --since <seq>` semantics | strictly incremental, ascending `seq`, payload carries the **full issue snapshot** |
| Single `bd events tail` latency | p50 = 65 ms / p95 = 122 ms |
| End-to-end (mutation → WS frame at the client) | p95 = 330 ms measured with a 200 ms poll interval in the spike; production code uses a 300 ms poll interval with a 200 ms per-call timeout (`POLL_INTERVAL` / `EVENTS_TAIL_TIMEOUT` in `task_events.rs` / `beads_client.rs`), which lands at p95 ≈ 500 ms |
| Claim atomicity | single winner; loser gets `issue already claimed by <actor>` |
| Idempotent re-claim | same actor re-claim is a no-op success |
| `bd --json` output shape | `create` = single object; `list`/`show`/`update`/`close` = arrays |
| Comments | `bd comment <id> <text> --json`; `bd show --include-comments --json` inlines them |

### Pitfalls the spike found

- **`bd events tail <N>` positional argument is silently ignored** — the poll
  loop must use the `--since <N>` flag or it re-reads from the start.
- **`bd -C <dir>` requires `<dir>` to already be a beads workspace** —
  `bd init` must run with cwd = the project directory instead.
- **Journal is per-replica** — after `bd dolt pull` from another machine the
  local journal does not contain the remote mutations; the board needs a
  full re-baseline (`bd list --json`) + cursor reset to the journal head.

## Environment

- `bd` 1.3.4 at `/usr/sbin/bd`, embedded Dolt backend (no external server).
- Global state (`~/.beads`, `~/.config/bd`, `~/.dolt`) is resolved via `HOME`.

## Re-verified while landing relay 3 (2026-11-24)

- Concurrent `bd update <id> --claim` from two processes: exactly one exits 0,
  the other exits 1 with `issue already claimed by <actor>` — the WebUI maps
  that to HTTP 409 naming the holder (`tests/testsuite_acceptance_test.rs`,
  `task_board_lifecycle_and_claim_race_journey`).
- `bd update <id> --add-label in-review` succeeds regardless of the actor
  (the session-terminal hook's actor is the bound agent kind), and a
  `--add-label` update does **not** change `status`.
- `bd assign <id> <name>` sets the assignee **without** moving `status` off
  `open` — `in_review` is therefore "in-review label on an `in_progress`
  issue", and the board's `In progress` column is driven by `status`.
- **`bd init` discovers ancestor `.beads` directories.** Run inside the sebas
  checkout (e.g. under `target/tests/`) it aborts with "This workspace is
  already initialized" and every subsequent `bd -C` call would write into the
  operator's real repository database. Task-center sandboxes therefore put
  the project directory **outside the repo** (`/tmp/sebas-task-center-*`) and
  pin `SEBAS_WORKSPACE_ROOT` there. This is the isolation red line (task 11.3).
