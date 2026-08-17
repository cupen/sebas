# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
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
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Build & Test

_Add your build and test commands here_

```bash
# Example:
# npm install
# npm test
```

## Architecture Overview

_Add a brief overview of your project architecture_

### Provider 管理（`/provider` 命令，bead sebas-63f epic）

- **运行态**：`ProviderMode`（`Off` / `Direct { provider }` / `Gateway`）持久化在 `~/.sebas/state.json`，读写逻辑在 `router/src/provider_state.rs`。
- **数据**：provider 配置（base_url、api_key、default_model 等）存 `~/.sebas/providers.json`（overlay），schema / 表单 / 种子加载在 `src/provider.rs`。
- **驱动抽象**：`AgentDriver` trait + `ClaudeCodeDriver` 实现位于 `acp-claude/src/agent_driver.rs`，把 `ProviderResolution` 翻成 agent 进程 env / argv。
- **spawn 翻译**：中间环节在 `src/spawn_env.rs::compute_provider_resolution` —— mode → 解析 URL+密钥 → `ProviderResolution`，再喂给 driver 出 `(extra_env, extra_args)`。
- **UI**：单一「Provider 管理」主卡在 `router/src/router/provider_card.rs`（详见 `.claude/rules/how-to.md`）。
- **探测 model**：详情面板的「🔍 探测 model 列表」按钮是 best-effort 便利，优先试 openai-compatible `/models`，失败回退 `/v1/models`。

## Conventions & Patterns

_Add your project-specific conventions here_
