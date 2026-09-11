## Context

See `proposal.md` for motivation. Facts that shape the approach (verified in code):

- core already writes provider data to SQLite: `src/core_channel/server.rs` `providers_mutation` /
  `aliases_mutation` do read-modify-write through `DbStateEngine` (`src/sebas_state/repo.rs`),
  and the Feishu card reaches it via `port.state_mutate("providers", …)`.
- the router nevertheless writes too: `sebas-router/src/admin.rs` routes every mutation through
  `channel_write` **and falls back to `write_overlay_rmw` (providers.json) when the channel is
  absent or fails**; `put_defaults` writes `defaults.json` directly and core never reads it.
- the router reads at startup from the file and at runtime from either the channel subscription
  or an inotify watcher, merging both with the same `apply_overlay` semantics.
- `sebas-webui` has **no core-channel dependency**; `routes.rs` builds a `RouterClient` and speaks
  plain HTTP to the router process. The root binary assembles a `CoreChannelBackend` only for
  sessions (`webui_cmd.rs`).
- spawn-time `read_overlay_item` (`src/spawn_env.rs`) prefers the legacy `providers.json` file over
  the store, so a stale file can shadow the DB's `default_model`.

## Goals / Non-Goals

**Goals:**

- Core is the only writer of provider, model, alias and default data; the router cannot write.
- The WebUI reaches that data through core, not through the router process.

**Non-Goals:**

- Changing the provider/model field shape (see `redesign-provider-models-settings`).
- Re-implementing upstream model fetching (see `add-fetch-models`).
- Renaming the `/router/api/*` paths, which become a misnomer (see proposal Non-goals).
- Removing the router's read-only operator views (stats, metrics, preset table).

## Decisions

### D1. Management rides the existing core channel, not a new core HTTP port

Core's provider surface becomes the `providers` / `aliases` domains plus defaults folded into
`settings`. Clients use `StateSnapshot` / `StateMutation` / `StateSubscribe`.

*Alternative rejected*: a dedicated HTTP admin API on the core process. It would duplicate the
channel's secret + peer-uid authentication, introduce a second bind/port to secure and to keep out
of the real-instance ports, and both the Feishu card and the root binary already speak the channel.

### D2. The WebUI backend gets a core-channel provider client

`sebas-webui` gains access to the core channel (the same backend the root binary already builds for
sessions), and the `/router/api/*` handlers are fulfilled from it.

*Alternative rejected*: reads via the router HTTP proxy and writes via core. Two sources for one
dataset means a read immediately after a write can disagree; the provider page would flicker between
old and new values. One source removes the class.

*Consequence*: in a detached form with no reachable core, provider management degrades to an honest
503 instead of a stale snapshot.

### D3. Defaults move into the store

Default provider/model are written into the core store under `settings`, committed with provider
data.

*Alternative rejected*: keep `defaults.json`. It is currently a router-owned file core never reads,
which is exactly the second-authority problem this change removes.

*Migration*: unlike provider data, `defaults.json` holds a user choice that would be silently lost.
It is imported **once** on first core start into the `settings` domain and then ignored, with a log
line. This is a deliberate exception to "legacy JSON is not imported", justified because defaults
are not provider identity data.

### D4. Router admin surface shrinks to read-only

Provider/alias/defaults CRUD and probe are deleted from `sebas-router/src/admin.rs`; the admin
router keeps authentication, stats, metrics, the static preset table, and the external hot-reload
path. A mutation attempt against the router now 404s.

*Alternative rejected*: keep the routes as 503 stubs. A route that exists but can never succeed
misleads callers and keeps a dead code path alive; a 404 states the surface is gone.

### D5. Stop preferring the legacy file at spawn

`read_overlay_item` stops preferring `providers.json`; the store is authoritative. The file remains
readable only on the existing no-core-store degradation path.

*Alternative rejected*: leave it for compatibility. It is a live correctness bug today: a stale
file pins the wrong `default_model` for Direct spawns while the DB holds the current value.

## Risks / Trade-offs

- [Detached WebUI loses provider management] no core channel means no provider surface. →
  Mitigation: the existing `Honest degradation when the core is unreachable` posture covers it; the
  page reports the source unavailable instead of showing stale rows.
- [One-time defaults import is a special case] it contradicts the "no legacy import" rule. →
  Mitigation: scoped to `defaults.json` only, logged, and stated in the requirement's migration
  note so it is not mistaken for a general import path.
- [Removing router writes breaks external scripts] anything calling `/admin/providers*` directly
  stops working. → Mitigation: this is the point of the change; the migration target is the core
  channel, and the removed requirements carry explicit Migration notes.
- [Rollback] an older binary restores the router write path; the core store stays readable, so data
  is not lost, but writes made by the newer binary remain authoritative. → Mitigation: no data
  format change is introduced, so downgrade is data-safe.

## Migration Plan

1. Land the core surface first (defaults into `settings`, provider/alias management validated in
   core) so nothing depends on a router write.
2. Rewire the WebUI backend to core, verify provider read/write, then delete the router's mutation
   routes and file fallback.
3. Import `defaults.json` once into `settings` on first core start; then stop reading the file.
4. Drop the spawn-time legacy-file preference.
5. Rollback: revert the binary. The store is untouched in shape, and the old router write path
   resumes; no downgrade step is required.

## Open Questions

- Whether the `/router/api/*` paths should be renamed to something core-neutral is deferred; it is
  cosmetic and touching it would collide with the frontend work in
  `redesign-provider-models-settings`.
