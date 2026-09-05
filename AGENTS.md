# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git
> remote — separate from `refs/heads/*` where your code lives.
> `.beads/issues.jsonl` is a passive export, not the wire protocol.
>
> See [SYNC_CONCEPTS.md](https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the
> source of truth; don't `bd import` during normal operation; don't
> reach for third-party Dolt hosting before trying the default).

## Git Workflow

- New features are developed on `feat/*` branches, never directly on `main`.
- Commit messages are a single sentence following
  [Conventional Commits](https://www.conventionalcommits.org/zh-hans/v1.0.0-beta.4/).
- Merging a `feat/*` branch back to `main`:
  1. Rebase the branch onto `main` first, then merge with `--no-ff`.
  2. Exception: if the branch has few commits and adds no new feature, just
     rebase onto `main` and fast-forward it into `main` (no merge commit).

## Frontend/Backend Integration Testing (联调)

Backend changes count as done only after they are verified against the real
frontend. Division of labor:

- **Frontend**: the operator runs `pnpm run dev` in `sebas-webui/frontend`
  (Vite on `127.0.0.1:5273`, strictPort). HMR auto-applies frontend edits —
  never start/stop/reconfigure that dev server yourself. If you need the
  frontend running for your own verification, use the separate
  `pnpm run dev:sandbox` script instead — `dev` belongs to the operator.
- **Backend**: build and run it yourself (`cargo build`), in a **sandbox**.

### Sandbox rules (never touch the operator's real instance)

The operator's real sebas (AppImage, port **9797**, real `~/.sebas` /
`~/.config/sebas` / provider credentials) is off-limits: do not restart it,
do not bind its ports, do not read or copy its credentials, and never point
sandbox processes at its files.

**WebUI sandbox shortcut**: `bash scripts/test_webui_sandbox.sh` spins up the
throwaway webui on port 9879 with auth **disabled by default** (login-free GUI
testing). `SANDBOX_AUTH=1` turns auth on with the unified test account
**admin / admin** (`webui-passwd` only warns on passwords shorter than 8
chars). Ctrl-C stops it and deletes the sandbox dir.

1. Every sandbox path goes under a throwaway dir (e.g. `/tmp/sebas-itest/`)
   and must override **all** of the defaults that otherwise fall back to the
   real `~/.sebas`:

   - config `-c` path (there is no sandbox-safe default), with
     `[dispatch] state_file`, `[media] download_dir`,
     `[acp.claude] sessions_dir` / `work_dir`,
     `[watchdog.core] channel_path`, and `[watchdog.webui]` host/port
     (pick a port ≠ 9797, e.g. 9877) all set inside it;
   - env: `SEBAS_CORE_SECRET=<fake>` (mimics the watchdog's injection; its
     presence is what arms the core session channel and the webui client),
     plus the three that default to the real `~/.sebas` files — all
     mandatory: `SEBAS_STATE_DB` (state store SQLite, default
     `~/.sebas/sebas.db` — the easy one to miss: without it the sandbox
     opens the real DB even with everything else sandboxed),
     `SEBAS_STATE_FILE` (provider state JSON, default
     `~/.sebas/state.json`), and `SEBAS_ROUTER_PROVIDER_OVERLAY`
     (default `~/.sebas/providers.json`).

2. Run the two halves exactly as the watchdog would:

   ```bash
   SEBAS_CORE_SECRET=fake SEBAS_STATE_DB=… SEBAS_STATE_FILE=… \
     SEBAS_ROUTER_PROVIDER_OVERLAY=… \
     target/debug/sebas core -c /tmp/sebas-itest/config.toml         # core
   SEBAS_CORE_SECRET=fake \
     target/debug/sebas webui -c /tmp/sebas-itest/config.toml        # webui
   ```

3. Verify over HTTP on the sandbox port (`/health`, `/api/summary`,
   `/api/sessions`, POST `/api/sessions` + `/{key}/message`), and/or open
   `http://127.0.0.1:<sandbox-port>/` in the browser — `cargo build` bakes
   the current `frontend/dist` into the binary, so the sandbox serves the
   real UI.

4. Clean up: SIGTERM the core (graceful exit removes the channel socket and
   dumps state — itself worth asserting), stop the webui, delete the sandbox
   dir, and confirm the ports are free.

### Sandbox debug recipe (proven end-to-end, agent-runnable)

一键自动化：本菜谱已固化为进程级 e2e 套件——`invoke e2e`（构建 + 跑
`tests/core_flow_e2e_test.rs`；单用例 `invoke e2e --case <name>`）或
`cargo test --test core_flow_e2e_test -- --ignored`。验收套件（旅程级，
覆盖面账本见 `tests/acceptance/COVERAGE.md`）：`invoke accept` 或
`cargo test --test acceptance_suite_test -- --ignored`。

`--debug` makes the router inject a built-in `test` provider that answers
requests itself (fixed text + echo, no upstream dial, downstream auth
skipped), and pointing `[acp.claude] path` at the `tests/bin` fake-claude
stub (built by `cargo build` together with the main binary) lets ACP
sessions complete full turns with zero real credentials. Let `<SB>` be the
throwaway dir (e.g. `/tmp/sebas-debug`).

1. `mkdir -p` the dirs the config references — especially `work_dir`, which
   must **exist on disk before any ACP session spawns** — then write
   `<SB>/config.toml`:

   ```toml
   [feishu]
   enabled = false

   [acp.claude]
   path = "<repo>/target/debug/fake-claude.exe"   # no .exe suffix on unix
   sessions_dir = "<SB>/claude-sessions"
   work_dir = "<SB>/work"

   [dispatch]
   state_file = "<SB>/sessions.json"

   [media]
   download_dir = "<SB>/downloads"

   [watchdog.core]
   channel_path = "<SB>/core-channel.sock"

   [watchdog.webui]
   enabled = false          # bare core owns the webui via --webui-port

   # router validate requires ≥1 provider with a base_url — the debug
   # `test` provider is injected only AFTER parse, so it cannot satisfy
   # validate. This dummy never dials anything in debug mode.
   [provider.anthropic]
   api_key = "sk-sandbox-dummy"

   [router]
   provider_overlay = "<SB>/providers.json"   # missing file = no-op
   usage_file = "<SB>/router-usage.jsonl"
   ```

2. Start the core in debug mode:

   ```bash
   cargo build
   SEBAS_CORE_SECRET=fake SEBAS_STATE_DB="<SB>/sebas.db" \
     SEBAS_STATE_FILE="<SB>/state.json" \
     SEBAS_ROUTER_PROVIDER_OVERLAY="<SB>/providers.json" \
     target/debug/sebas core -c "<SB>/config.toml" \
     --router --debug --webui --webui-port 9877 > "<SB>/core.log" 2>&1
   ```

   The built-in router binds a random port — read it from the log line
   `router started (core --router) … addr=127.0.0.1:<port>`.

3. Verify:
   - `GET http://127.0.0.1:9877/health` → `ok`; `/api/summary` →
     `reachability.ok = true` and `execution_bodies` shows acp `ok: true`
     (`native` stays `ok: false` in the sandbox — it needs
     `SEBAS_AGENT_PROVIDER_API_KEY`; report that honestly, don't fix).
   - router `POST /v1/messages` with `{"model":"test",…}` → 200
     `msg_test_debug`; the namespace form `test/<anything>` routes there too.
   - round-trip: `POST /api/sessions` with `{"prompt":"hello","backend":"acp"}`
     — that is the real request shape; `{"channel","message"}` is silently
     deserialized into an empty placeholder session. Then
     `GET /api/sessions/<encoded_key>` → fake-claude's "hello world", status
     Done.
   - Windows Git Bash curl gotcha: non-ASCII text in `-d '…'` is sent as
     GBK, the router's JSON parse fails, model extraction yields none, and
     the request dies with a misleading 502 `no_route` **even though
     routing is fine** (check `/admin/stats` — `routes: 1` means the debug
     route is present). Use ASCII payloads or `-d @file.json` (UTF-8).

4. Clean up per rule 4 — every sandbox artifact (incl. the state DB) lives
   inside `<SB>`, so deleting the dir is complete.

### What a sandbox can and cannot verify

Verifiable: route surface, channel/socket lifecycle (appears while running,
removed on graceful exit), webui↔core connect/reconnect (`reachability` in
`/api/summary` flips `ok`/`cause`), spawn/message round-trips, typed
rejections, wrong-secret refusal. With the fake-claude stub as the ACP agent
(recipe above), a full session turn completes end-to-end — against synthetic
answers, not a real model. **Not** verifiable without the operator's
provider credentials: a REAL ACP child completing a turn — without the stub,
sessions spawn then the child dies honestly; do not interpret that as a
channel failure.
Report such limits explicitly instead of marking the task done.

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:970c3bf2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   bd dolt push
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
