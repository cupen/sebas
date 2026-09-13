# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>           # Complete work
bd dolt push            # Push beads data to remote
```

> **Architecture in one line:** issues live in a local Dolt DB (`.beads/dolt/`);
> cross-machine sync is `bd dolt push/pull` over `refs/dolt/data` on your git
> remote — separate from `refs/heads/*`. `.beads/issues.jsonl` is a passive
> export, not the wire protocol. One-screen overview and anti-patterns (JSONL
> is not the source of truth; no `bd import` in normal operation; try the
> default sync before third-party Dolt hosting):
> [SYNC_CONCEPTS.md](https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md).

## Git Workflow

- New features go on `feat/*` branches, never directly on `main`.
- Commit messages follow
  [Conventional Commits](https://www.conventionalcommits.org/zh-hans/v1.0.0-beta.4/)
  and must be **one concise sentence** summarizing the change — no long-winded
  bodies or stacked detail clauses; details belong in issues/PRs, not commits.
- Merging a `feat/*` branch back to `main`:
  1. Rebase onto `main` first, then merge with `--no-ff`.
  2. Exception: few commits and no new feature → rebase onto `main` and
     fast-forward (no merge commit).

## Frontend/Backend Integration Testing (联调)

Backend changes count as done only after verification against the real
frontend. Division of labor:

- **Frontend**: the operator runs `pnpm run dev` in `sebas-webui/frontend`
  (Vite on `127.0.0.1:5273`, strictPort); HMR auto-applies edits — never
  start/stop/reconfigure that server yourself. For your own verification use
  `pnpm run dev:sandbox` instead.
- **Backend**: build and run it yourself (`cargo build`), in a **sandbox**.

### Sandbox rules (never touch the operator's real instance)

The operator's real sebas (AppImage, port **9797**, real `~/.sebas` /
`~/.config/sebas` / provider credentials) is off-limits: never restart it,
bind its ports, read/copy its credentials, or point sandbox processes at its
files.

**WebUI sandbox shortcut**: `invoke testsuite-webui-sandbox` spins up a
throwaway webui on port 9879 with auth **disabled by default** (login-free GUI
testing); `invoke testsuite-webui-sandbox --auth` turns auth on with the test
account **admin / admin**, provisioned into the sandbox-local
`SEBAS_WEBUI_AUTH_DB` (add-webui-multiuser-rbac) via
`sebas webui-passwd --user admin --password-stdin` — the first user in a
fresh user store defaults to the root role (`webui-passwd` only warns on
passwords shorter than 8 chars; the interactive `--auth` login prompt is
username `admin` + password `admin`). Ctrl-C stops it and deletes the sandbox
dir. The same assembly serves
Playwright (`invoke testsuite-webui-server` on port 9899, 9898 with `TESTSUITE_AUTH=1`);
both live in tasks.py — the old `scripts/*sandbox*.sh` harnesses were removed
(their configs used stale keys the current binary rejects, e.g.
`[acp.claude]`, `[router] state_file`).

1. Keep every sandbox path under one throwaway dir (e.g. `/tmp/sebas-itest/`)
   and override **all** defaults that would fall back to the real `~/.sebas`:

   - config `-c` path (no sandbox-safe default exists), with
     `[dispatch] state_file`, `[media] download_dir`,
     `[acp.claude] sessions_dir` / `work_dir`,
     `[watchdog.core] channel_path`, and `[watchdog.webui]` host/port
     (port ≠ 9797, e.g. 9877) all set inside it — decide the auth posture
     explicitly: `auth = false` for login-free sandboxing, or keep the
     default-on switch and provision a user into a sandbox-local
     `SEBAS_WEBUI_AUTH_DB` (below). Left alone with zero users, a loopback
     webui stops at the first-run setup page (whoever hits it first creates
     root) and the user store would land in the real `~/.sebas`;
   - env: four files that would otherwise default into the real `~/.sebas`
     — all mandatory: `SEBAS_STATE_DB` (SQLite, default `~/.sebas/sebas.db`
     — the easy one to miss: without it the sandbox opens the real DB even
     with everything else sandboxed), `SEBAS_STATE_FILE` (default
     `~/.sebas/state.json`), `SEBAS_ROUTER_PROVIDER_OVERLAY` (default
     `~/.sebas/providers.json`), and `SEBAS_WEBUI_AUTH_DB` (WebUI user
     store, default `~/.sebas/auth.db`; nothing opens it while
     `auth = false`, pin it anyway so any provisioning stays sandbox-local).
     Do **not** set `SEBAS_CORE_SECRET`: the core
     auto-arms (generates a key, writes it to `<config dir>/core.secret`,
     0600) and clients discover it from that file on every connect attempt —
     the env var is only for explicitly simulating a wrong-secret refusal.

   Provisioning a WebUI login user (only needed when `auth` stays on):
   either `sebas webui-passwd --user <name> [--password-stdin|--password]
   [--role root|admin|member|viewer]` against the pinned
   `SEBAS_WEBUI_AUTH_DB` (first user defaults to root, later ones to
   member), or export `SEBAS_WEBUI_USER` + `SEBAS_WEBUI_PASSWORD` before
   starting the webui (bootstraps root at first start, idempotent on
   restart). Login is always username + password — the retired single-field
   token / JSON credentials file forms no longer exist.

2. Run the two halves exactly as the watchdog would:

   ```bash
   SEBAS_STATE_DB=… SEBAS_STATE_FILE=… \
     SEBAS_ROUTER_PROVIDER_OVERLAY=… \
     target/debug/sebas core -c /tmp/sebas-itest/config.toml         # core
   SEBAS_STATE_DB=… SEBAS_STATE_FILE=… \
     SEBAS_ROUTER_PROVIDER_OVERLAY=… \
     target/debug/sebas webui -c /tmp/sebas-itest/config.toml        # webui
   ```

3. Verify over HTTP on the sandbox port (`/health`, `/api/summary`,
   `/api/sessions`, `POST /api/sessions` + `/{key}/message`), and/or open
   `http://127.0.0.1:<sandbox-port>/` — `cargo build` bakes the current
   `frontend/dist` into the binary, so the sandbox serves the real UI.

4. Clean up: SIGTERM the core (graceful exit removes the channel socket and
   dumps state — itself worth asserting), stop the webui and the standalone
   router process, delete the sandbox dir, and confirm the ports are free.

### Sandbox debug recipe (proven end-to-end, agent-runnable)

一键自动化：本菜谱已固化为进程级 e2e 套件——`invoke testsuite-e2e`（构建 + 跑
`tests/testsuite_e2e_test.rs`；单用例 `invoke testsuite-e2e --case <name>`）或
`cargo test --test testsuite_e2e_test -- --ignored`。验收套件（旅程级，覆盖面
见 `tests/acceptance/COVERAGE.md`）：`invoke testsuite-acceptance` 或
`cargo test --test testsuite_acceptance_test -- --ignored`。

`--debug` makes the router inject a built-in `test` provider that answers
itself (fixed text + echo, no upstream dial, downstream auth skipped), and
pointing `[acp.claude] path` at the `tests/bin` fake-claude stub (built by
`cargo build`) lets ACP sessions complete full turns with zero real
credentials. Let `<SB>` be the throwaway dir (e.g. `/tmp/sebas-debug`).

1. `mkdir -p` every dir the config references — `work_dir` must **exist on
   disk before any ACP session spawns** — then write `<SB>/config.toml`:

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
   auth = false             # 登录免了：零用户 + 默认开的 auth 会停在首启
                            # 设置页，且 auth.db 落进真实 ~/.sebas——沙箱内
                            # 要么显式关，要么把 SEBAS_WEBUI_AUTH_DB 钉进
                            # 沙箱并建户（webui-passwd / env 引导）

   # router validate requires ≥1 provider with a base_url — the debug `test`
   # provider is injected only AFTER parse, so it cannot satisfy validate.
   # This dummy never dials anything in debug mode.
   [provider.anthropic]
   api_key = "sk-sandbox-dummy"

   [router]
   provider_overlay = "<SB>/providers.json"   # missing file = no-op
   usage_file = "<SB>/router-usage.jsonl"
   ```

2. Start the two processes（unify-router-process-shape：router 只以独立进程
   `sebas router --config <path> [--debug]` 运行，core 旗标里没有 router）
   with no `SEBAS_CORE_SECRET` — auto-arm writes the generated key to
   `<SB>/core.secret` and clients discover it:

   ```bash
   cargo build
   SEBAS_STATE_DB="<SB>/sebas.db" \
     SEBAS_STATE_FILE="<SB>/state.json" \
     SEBAS_ROUTER_PROVIDER_OVERLAY="<SB>/providers.json" \
     target/debug/sebas core -c "<SB>/config.toml" \
     --webui --webui-port 9877 > "<SB>/core.log" 2>&1
   SEBAS_STATE_DB="<SB>/sebas.db" \
     SEBAS_STATE_FILE="<SB>/state.json" \
     SEBAS_ROUTER_PROVIDER_OVERLAY="<SB>/providers.json" \
     target/debug/sebas router -c "<SB>/config.toml" \
     --debug > "<SB>/router.log" 2>&1 &
   ```

   Router 地址来自独立进程日志：读 `<SB>/router.log` 里的
   `sebas router listening addr=127.0.0.1:<port>`（想钉固定端口就在 config
   的 `[router] listen` 配一个；默认 `127.0.0.1:8787` 是固定值，别与操作员
   实例的托管 router 相撞）。

3. Verify:
   - `GET /health` → `ok`; `/api/summary` → `reachability.ok = true` and
     `execution_bodies` shows acp `ok: true` (`native` stays `ok: false` in
     the sandbox — it needs `SEBAS_AGENT_PROVIDER_API_KEY`; report that
     honestly, don't fix).
   - router `POST /v1/messages` with `{"model":"test",…}` → 200
     `msg_test_debug`; the namespace form `test/<anything>` routes there too.
   - round-trip: `POST /api/sessions` with `{"prompt":"hello","backend":"acp"}`
     (the real request shape — `{"channel","message"}` is silently
     deserialized into an empty placeholder session), then
     `GET /api/sessions/<encoded_key>` → fake-claude's "hello world", status
     Done.
   - Windows Git Bash curl gotcha: non-ASCII text in `-d '…'` is sent as GBK,
     the router's JSON parse fails, and the request dies with a misleading
     502 `no_route` **even though routing is fine** (check `/admin/stats`:
     `routes: 1` means the debug route is present). Use ASCII payloads or
     `-d @file.json` (UTF-8).

4. Clean up per rule 4 — every artifact (incl. the state DB) lives inside
   `<SB>`, so deleting the dir is complete.

### What a sandbox can and cannot verify

Verifiable: route surface; channel/socket lifecycle (present while running,
removed on graceful exit); webui↔core connect/reconnect (`reachability` in
`/api/summary` flips `ok`/`cause`); spawn/message round-trips; typed
rejections; wrong-secret refusal (set `SEBAS_CORE_SECRET` explicitly on one
side only). With the fake-claude stub as the ACP agent,
a full session turn completes end-to-end — against synthetic answers, not a
real model. **Not** verifiable without the operator's provider credentials: a
REAL ACP child completing a turn — without the stub, sessions spawn then the
child dies honestly; that is not a channel failure. Report such limits
explicitly instead of marking the task done.

## Non-Interactive Shell Commands

`cp`, `mv`, and `rm` may be aliased to `-i` (interactive) mode on some
systems, hanging the agent on a y/n prompt. **Always use non-interactive
forms:**

```bash
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

Other commands that may prompt: `scp` / `ssh` → `-o BatchMode=yes`;
`apt-get` → `-y`; `brew` → `HOMEBREW_NO_AUTO_UPDATE=1`.

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


<!-- rtk-instructions v2 -->
# RTK (Rust Token Killer) - Token-Optimized Commands

## Golden Rule

**Always prefix commands with `rtk`**. If RTK has a dedicated filter, it uses it. If not, it passes through unchanged. This means RTK is always safe to use.

**Important**: Even in command chains with `&&`, use `rtk`:
```bash
# ❌ Wrong
git add . && git commit -m "msg" && git push

# ✅ Correct
rtk git add . && rtk git commit -m "msg" && rtk git push
```

## RTK Commands by Workflow

### Build & Compile (80-90% savings)
```bash
rtk cargo build         # Cargo build output
rtk cargo check         # Cargo check output
rtk cargo clippy        # Clippy warnings grouped by file (80%)
rtk tsc                 # TypeScript errors grouped by file/code (83%)
rtk lint                # ESLint/Biome violations grouped (84%)
rtk prettier --check    # Files needing format only (70%)
rtk next build          # Next.js build with route metrics (87%)
```

### Test (60-99% savings)
```bash
rtk cargo test          # Cargo test failures only (90%)
rtk go test             # Go test failures only (90%)
rtk jest                # Jest failures only (99.5%)
rtk vitest              # Vitest failures only (99.5%)
rtk playwright test     # Playwright failures only (94%)
rtk pytest              # Python test failures only (90%)
rtk rake test           # Ruby test failures only (90%)
rtk rspec               # RSpec test failures only (60%)
rtk test <cmd>          # Generic test wrapper - failures only
```

### Git (59-80% savings)
```bash
rtk git status          # Compact status
rtk git log             # Compact log (works with all git flags)
rtk git diff            # Compact diff (80%)
rtk git show            # Compact show (80%)
rtk git add             # Ultra-compact confirmations (59%)
rtk git commit          # Ultra-compact confirmations (59%)
rtk git push            # Ultra-compact confirmations
rtk git pull            # Ultra-compact confirmations
rtk git branch          # Compact branch list
rtk git fetch           # Compact fetch
rtk git stash           # Compact stash
rtk git worktree        # Compact worktree
```

Note: Git passthrough works for ALL subcommands, even those not explicitly listed.

### GitHub (26-87% savings)
```bash
rtk gh pr view <num>    # Compact PR view (87%)
rtk gh pr checks        # Compact PR checks (79%)
rtk gh run list         # Compact workflow runs (82%)
rtk gh issue list       # Compact issue list (80%)
rtk gh api              # Compact API responses (26%)
```

### JavaScript/TypeScript Tooling (70-90% savings)
```bash
rtk pnpm list           # Compact dependency tree (70%)
rtk pnpm outdated       # Compact outdated packages (80%)
rtk pnpm install        # Compact install output (90%)
rtk npm run <script>    # Compact npm script output
rtk npx <cmd>           # Compact npx command output
rtk prisma              # Prisma without ASCII art (88%)
rtk uv run <cmd>        # Compact uv project command output
```

### Files & Search (60-75% savings)
```bash
rtk ls <path>           # Tree format, compact (65%)
rtk read <file>         # Code reading with filtering (60%)
rtk grep <pattern>      # Search grouped by file (75%). Format flags (-c, -l, -L, -o, -Z) run raw.
rtk find <pattern>      # Find grouped by directory (70%)
```

### Analysis & Debug (70-90% savings)
```bash
rtk err <cmd>           # Filter errors only from any command
rtk log <file>          # Deduplicated logs with counts
rtk json <file>         # JSON structure without values
rtk deps                # Dependency overview
rtk env                 # Environment variables compact
rtk summary <cmd>       # Smart summary of command output
rtk diff                # Ultra-compact diffs
```

### Infrastructure (85% savings)
```bash
rtk docker ps           # Compact container list
rtk docker images       # Compact image list
rtk docker logs <c>     # Deduplicated logs
rtk kubectl get         # Compact resource list
rtk kubectl logs        # Deduplicated pod logs
```

### Network (65-70% savings)
```bash
rtk curl <url>          # Compact HTTP responses (70%)
rtk wget <url>          # Compact download output (65%)
```

### Meta Commands
```bash
rtk gain                # View token savings statistics
rtk gain --history      # View command history with savings
rtk discover            # Analyze Claude Code sessions for missed RTK usage
rtk proxy <cmd>         # Run command without filtering (for debugging)
rtk init                # Add RTK instructions to CLAUDE.md
rtk init --global       # Add RTK to ~/.claude/CLAUDE.md
```

## Token Savings Overview

| Category | Commands | Typical Savings |
|----------|----------|-----------------|
| Tests | vitest, playwright, cargo test | 90-99% |
| Build | next, tsc, lint, prettier | 70-87% |
| Git | status, log, diff, add, commit | 59-80% |
| GitHub | gh pr, gh run, gh issue | 26-87% |
| Package Managers | pnpm, npm, npx | 70-90% |
| Files | ls, read, grep, find | 60-75% |
| Infrastructure | docker, kubectl | 85% |
| Network | curl, wget | 65-70% |

Overall average: **60-90% token reduction** on common development operations.
<!-- /rtk-instructions -->
