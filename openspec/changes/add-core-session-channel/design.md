## Context

Three facts, each verified in the source, determine this design:

1. `src/webui_cmd.rs:82` restores the session map from the state file **once, at
   boot**, and `webui_cmd.rs:93` drops the outbound receiver
   (`let (router, _out_rx) = ...`).
2. `src/run.rs:311` is the **only** writer of that state file, and it runs in the
   shutdown path, after the WS loop exits.
3. `openspec/specs/webui/spec.md` calls `run --webui` the **legacy** path, and the
   ownership guard refuses to run it alongside the watchdog-spawned WebUI.

Together: the production WebUI renders the session set as of the previous core
exit, for the entire lifetime of the current core run, and can mutate nothing.
The one path that can drive is the one the spec deprecates.

## Goals / Non-Goals

**Goals**: one authority for session state; a detached WebUI that is live and
drivable; a seam that makes the WebUI crate testable without a core; honest
behavior when the core is down.

**Non-Goals**: remote access; multi-user auth; changing the state file format or
when it is written; hosting ACP children anywhere but the core; project-unit UI
(that is `add-project-workbench`).

## Decisions

### D1: A core-owned socket, not the watchdog control RPC

The obvious shortcut is to extend `RpcControlRequest` with session methods, since
the WebUI already speaks that protocol for admin actions. Rejected: the watchdog
is a **supervisor**. It holds `ManagedService` lifecycle, updates, and rollback —
it does not hold a `RouterHandle` and has no access to session state. Adding
session methods there would mean the watchdog relaying to the core over a channel
that does not exist, so the channel has to be built regardless; routing it through
the supervisor only adds a hop and blurs which process owns what.

So: a second socket, owned by the core, with the same security posture and framing
as the control RPC. `~/.sebas/core.sock`, mode 0600.

### D2: The WebUI is a client, not a replica

The smaller diff would keep the WebUI's local `RouterHandle` and use the socket to
sync it — forward `Out::WebSpawn` outward, apply core events inward. It barely
touches `routes.rs`.

Rejected, and this is the load-bearing decision. A synced replica has two copies
of mutable session state and therefore a divergence mode with no resolution rule:
when the local router and the core disagree, nothing decides. Worse, the local
router would remain *capable* of mutating, so any code path that forgot to go
through the socket would silently write to the wrong authority — the exact class
of bug this change exists to remove.

The WebUI holds a cache with no write path and no authority. The cost is real —
`routes.rs` is rewritten off `RouterHandle` — and it buys a property that is
otherwise unbuyable.

### D3: Framing — newline-delimited JSON, request/response plus one stream

Same wire shape as the control RPC: one JSON object per line. Most methods are
request/response. `subscribe` is different: it responds with a snapshot, then the
connection stays open and the core pushes event lines until the client goes away.

A subscriber therefore holds a dedicated connection; mutations use short-lived
ones. That avoids interleaving a long-lived push stream with responses on the same
socket, which would otherwise need request ids and a correlation table.

### D4: Auth — peer uid **and** secret, both required

Peer uid equality via `SO_PEERCRED`, plus a shared secret in the first line, as
`SEBAS_CORE_SECRET`, injected by the watchdog into both core and WebUI the way
`SEBAS_CONTROL_SECRET` already is.

Both, not either. The uid check is the real boundary — this socket can spawn
processes, which makes it a privilege boundary and not merely an API. The secret
is defense in depth for the case where something else runs as the same uid, which
on a single-user box is most things.

`project_dir` is canonicalized and stat'd before any spawn. A create request is
not a filesystem probe: rejection says "not a usable directory", not whether the
path exists.

### D5: Event source — a broadcast on the router

The router has no broadcast today; the WebUI's `broadcast::Sender<WebUiEvent>` in
`server.rs:47` is WebUI-local and, per the console change's audit, nothing
publishes to it. So the event source has to be created, not merely connected.

Add `broadcast::Sender<SessionEvent>` to the router, published on every mapping
mutation. Then both backends subscribe to the same source: the in-process backend
directly, the socket backend after a relay through the channel. The SSE stream to
the browser becomes the third hop of one chain rather than a separate mechanism —
which is why `run --webui` and `sebas webui` can be made to behave identically
instead of merely similarly.

Snapshot-then-stream on one subscription, in that order, closes the gap where an
event fires between a separate snapshot call and a separate subscribe call.

### D6: The seam — `trait SessionBackend`, shaped like `AdminAdapter`

`webui/src/admin.rs:41` already establishes the pattern: a trait in the WebUI
crate, implemented in the binary crate, `Option`al in state, absent means degraded.
Reuse it exactly — a second pattern for the same problem would be the worse
outcome.

Unlike `AdminAdapter` the backend is **not** optional: there is always a backend,
and it is the backend that reports whether the core is reachable. "No backend"
would reintroduce a silent-no-op path.

`webui::run*` takes `Arc<dyn SessionBackend>` where it took `RouterHandle`.
`routes.rs`, `sse.rs`, and `server.rs` follow. `run.rs` passes the in-process
implementation; `webui_cmd.rs` passes the socket client and stops building a
router at all.

### D7: Degradation is a first-class state, not an error page

Core unreachable is normal — the watchdog restarts the core and the WebUI is
specified to survive it. So it is a rendered state: the board says the core is not
connected and why, the composer is disabled with that reason, and nothing reports
a success it did not achieve.

The backend reconnects with backoff and re-snapshots on return, so recovery needs
no reload. This is what replaces the `can_drive: bool` flag from
`add-project-workbench`'s earlier draft: the condition is transient, its cause is
true, and the remedy is visible — rather than a permanently-off control with a
deferred-change excuse.

### D8: Relationship to the other two changes

- `redesign-webui-console` is unaffected and still archives first. Its "Live
  session board" requirement is only *satisfiable in production* once this lands;
  this change supplies the events its SSE client consumes.
- `add-project-workbench` archives last and drops `can_drive`, taking drivability
  as given and using this channel's turn-content method for its stream.

## Risks / Trade-offs

- **A socket that spawns processes is a privilege boundary.** → 0600, uid
  equality, secret, canonicalized and stat'd `project_dir`, typed rejections that
  do not leak path existence. Reviewed as a security surface, not as an API.
- **`routes.rs` rewritten off `RouterHandle` is the largest diff in the three
  changes.** → Land the trait and the in-process implementation first, with the
  legacy path green, before writing the socket implementation. Two verifiable
  halves rather than one unverifiable whole.
- **Existing WebUI tests construct a `RouterHandle`.** → A fake backend in the
  WebUI crate; the tests get simpler, since they no longer need a session manager.
- **Subscribers holding a connection each.** → One per WebUI process, not per
  browser tab; browser fan-out stays in the existing SSE broadcast.
- **A slow subscriber could lag the broadcast.** → Bounded channel; on lag, drop
  the subscription and force a fresh snapshot rather than delivering a gap
  silently.
- **Two sockets to reason about (control + core).** → Accepted per D1. Documented
  by owner: watchdog owns lifecycle, core owns sessions.
