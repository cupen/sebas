## Context

See proposal.md — Why. Current state (`src/service.rs`, `src/cli.rs`,
`openspec/specs/cli-service/spec.md`): `service --install` renders a systemd
system unit whose `ExecStart` is `<current_exe()> run --config <abs-config>`,
requiring EUID 0, non-root `--user`, exits 2–6, `--force`/`--auto-start`.

Two facts drive every decision here:

- The watchdog is the full runtime model: it supervises core/gateway/webui,
  owns the control socket + `services.json` desired-state, and performs
  self-upgrade. A systemd unit that runs bare `run` forfeits all of it.
- `upgrade::install_version`/`rollback` maintain a `data_dir/versions/vX` +
  `data_dir/current` symlink layout for updates. A unit that bakes the
  install-time `current_exe()` path decouples from that, so a post-update
  reboot reverts to the old binary (`upgrade.rs:71` `data_dir`, `:308`).

## Goals / Non-Goals

**Goals:**
- The installed unit runs the watchdog entrypoint, so services-installed
  `sebas` gets supervision, control plane, and self-upgrade for free.
- The unit's binary path stays stable across updates (uphill of
  `versions/`/`current` churn), and a reboot runs the latest installed
  version.

**Non-Goals:**
- Rearchitect `upgrade`'s `versions/current/rollback` layout — keep it as
  the history/rollback source; only add a stable-path seed.
- Add macOS launchd support; change watchdog internal spawn (it already
  reuses `current_exe()`, self-consistent once the watchdog itself runs from
  the stable path).

## Decisions

### D1: ExecStart runs `watchdog`, not `run`

Unit becomes `<fixed-binary> watchdog --config <config>`. systemd supervises
exactly one process (the watchdog); watchdog supervises the rest. Aligned
with webui-default launch direction.

### D2: Fixed binary path at `<data_dir>/bin/sebas`

- `data_dir` resolves from `config` `[watchdog.storage].data_dir` (config.rs
  `WatchdogConfig.storage.data_dir`, `:330`), else `~<user>/.local/share/sebas`
  (user home, **not** the installer's root home — the daemon must write/own
  it without root). Reuse `upgrade::data_dir`/`expand_tilde` (`upgrade.rs:71`,
  `:541`, made `pub`).
- Install seeds the current binary to `<data_dir>/bin/sebas` (new
  `upgrade::seed_stable_binary(path)` helper: version stamp + copy + chmod
  0755). `sebas update` replaces it in place → reboot runs the newest.
- Best-effort symlink `/usr/local/bin/sebas → <data_dir>/bin/sebas` for
  PATH convenience (no error if it can't be created).

**Alternatives considered:** bake `/usr/local/bin/sebas` owned by root — but
a non-root watchdog couldn't self-upgrade (rename fails), and it conflicts
with any future `ProtectSystem=strict`. chowning `/usr/local/bin` to the
service user is non-standard and weakens the system dir. `data_dir/bin`
keeps the on-disk layout self-owned and upgradeable.

### D3: No `ProtectSystem=strict`, yes `full`

`ProtectSystem=full` + `NoNewPrivileges` + `ProtectHome=read-only` +
`PrivateTmp` harden the unit. `strict` is deliberately **not** set: the
watchdog must be able to replace `<data_dir>/bin/sebas` on update; `strict`
would mount the whole `/` read-only including the (non-/usr) data dir — and
even `full` already keeps `/usr,/boot,/etc` read-only while leaving
`/home`,`/var` writable, which is exactly where the data + binary live.

### D4: Idempotent reload + restart

After writing + `daemon-reload`: if the unit is active, `systemctl restart`
so a repeated `install` deterministically applies new config; otherwise just
leave (and print enable hint when not `--auto-start`). Deterministic,
matches systemd's natural reload-vs-restart semantics.

### D5: `--log-level` bake source

`RUST_LOG` comes from `--log-level` when given, else the installing env,
falling back to `info` when unset/empty. New `--log-level` flag on
`ServiceArgs` (install-only; ignored by uninstall).

### D6: `--user` existence check

`validate_user` additionally rejects a `--user` that doesn't exist.
Use libc `getpwnam` (no external process) — matches existing `libc::geteuid`
dependency in the module.

### D7: systemd escaping

`ExecStart` binary+config args rendered via systemd quoting rules so paths
with whitespace/`;`/`%` stay a single token. Pure extension of
`render_unit`.

## Risks / Trade-offs

- [Non-root `data_dir/bin` is world-writable-adjacent] → keep perms 0755 and
  rely on the running user owning it; `ProtectHome=read-only` limits blast
  radius. Acceptable for a per-user install service.
- [`restart` on every install could interrupt a healthy daemon] → install is
  an explicit admin action; restart is the expected "take effect" semantic,
  and only fires when the unit is already active.
- [Seeding a new stable path adds a one-time copy on first install] → byte
  copy of a small binary; negligible.
- [Self-upgrade still needs the on-disk data_dir writable at runtime] →
  `ProtectSystem=full` keeps `/home`/data writable; verified against the
  hardening choice.

## Migration Plan

Existing installed units (running `run`) are unaffected until `--force`
re-install. Users migrate by `sudo sebas service --install --force
--auto-start`; the unit is rewritten to watchdog + stable path, seeded, and
restarted. `uninstall` path unchanged. No data migration. Rollback of the
change itself = reinstall an old build; `upgrade rollback` still restores the
pre-update binary via `versions/rollback`.

## Open Questions

None — all decisions above (D1–D7) were the material ambiguity flagged
during the interview that produced the locking answers (watchdog runtime,
fixed path). Remaining unknowns are implementation tact.