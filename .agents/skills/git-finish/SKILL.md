---
name: git-finish
description: "Git worktree wrap-up: commit work as conventional commits, rebase onto main for linear history, re-verify, push, and summarize. Explicit invocation ONLY: run /git-finish or ask for git-finish by name; never auto-trigger on 收尾 / 收工 / wrap up alone."
---

# Git 工作目录收尾（git-finish）

目标：会话结束时把工作区整理成**已提交、已集成（线性历史）、已验收、已推送**的状态，并以简短总结收束。用户调用本 skill 即视为对提交与推送的显式授权（覆盖默认的保守 git 策略），但只限本次调用范围内摸清的工作。

## 0. 摸清现场

```bash
git status --short                          # 改动与未跟踪文件
git branch --show-current                   # 当前分支
git log --oneline -5                        # 最近提交（了解既有提交风格）
git rev-parse --git-dir --git-common-dir    # 两者不同 = 在 worktree 里
```

- 查看仓库的 AGENTS.md / CLAUDE.md / README / CI 配置，确定两件事：**提交信息规范**（conventional commits、语言偏好）与**验证命令**（第 3 步的验收门禁用什么）。
- 在 worktree 中时注意：main 通常由主 worktree 持有，集成进 main 的操作（rebase 后快进 main）需在检出 main 的 worktree 执行（`git worktree list` 确认）。

## 1. 提交已完成的工作

- 先浏览 `git diff` 与未跟踪文件，确认改动内容与归属。构建产物、依赖目录、秘密文件（.env、密钥、凭据）**不提交**；发现与本会话无关的陌生改动时，单独成提交并在总结中标注，不与本次工作混装。
- 按逻辑层拆分 conventional commits，单句主题行（语言跟随仓库既有风格）。一次收尾通常 1–4 个提交，例如：后端协议、内核/服务、前端、文档各成一体。拆分让 review 与回滚都有着力点，但不要为了拆而拆。
- 没有任何改动 → 直接跳到第 2 步。

## 2. 集成（保持线性历史）

目标：main 的提交记录是一条直线，分支工作整体重放到最新 main 之上，不产生 merge commit。交错的历史难以追踪与回滚，线性历史让每个提交都落在明确的基线上。

**当前在 main**：

```bash
git fetch origin
git rebase origin/main
```

**当前在其它分支或 worktree**：

```bash
git fetch origin
git rebase origin/main   # 把整个分支重放到最新 main 之上；无远端时用本地 main
```

rebase 完成后把 main 快进到分支顶端：检出 main（或到主 worktree），执行

```bash
git merge --ff-only <branch>
```

rebase 之后分支已包含 main 的全部历史，`--ff-only` 必然成功；若失败说明 main 在此期间又前进了，回到 `git fetch` 重新 rebase 一轮，仍失败则停下报告。

- **默认不用 `--no-ff` merge**。仅当仓库有明确的 merge-based 约定（如 CI 依赖 merge commit）或用户点名要 merge 时才用。
- 解决冲突时警惕上游的**语义漂移**：上游可能新增了枚举变体、改了函数签名、重构了你要改的模块。机械地合并标记通过 ≠ 语义正确 —— 重放你的改动到上游新结构上，补上上游新增 case 的处理。
- rebase 会改写提交哈希：若分支此前推送过远端，本地会与远端分叉。本流程只推集成目标分支（第 4 步），不受影响；用户额外要求推工作分支时须明确同意 `--force-with-lease`。
- rebase 过程出问题可 `git rebase --abort` 回到起点重来，不硬撑。

## 3. 测试验收

集成（rebase / 快进）之后**必须重新验收** —— rebase 前的绿不代表 rebase 后的绿，上游语义变化可能静默破坏行为。

- 按第 0 步确定的仓库门禁跑测试与构建（Rust 仓库通常是 `cargo test` + `cargo build`；Node 仓库是 `pnpm test` + `pnpm build`；无明确门禁时至少构建通过）。
- 失败 → 修复后补提交，再回到第 2 步检查是否需要再 rebase。
- 无法修复 → **停下**，报告卡点，不推送。带着红测试推送比不推送更糟。

## 4. 推送

```bash
git push
```

- 被拒（non-fast-forward）→ `git fetch && git rebase origin/main` 后重试一次；再失败则停下报告。
- **不 force-push**，除非用户明说。
- 只推集成目标分支（通常 main，或用户指名的分支），不顺手推送无关分支。

## 5. 总结

简练、按重要性排列、不陷入细节。结论先行，工作 3–5 条以内，如实记录边界：

```text
收尾完成，已推送 main（<commit 范围概述>）。
- <最重要的工作，一句话>
- <次重要的工作>
- <再其次>
- 验证：<门禁结果；什么没验到，为什么>
- 遗留：<已知问题 / 后续建议，没有则不写>
```

边界记录是收尾的职责之一：哪些路径只被测试覆盖、哪些需要真实环境才能验证、哪些决定是当场拍的 —— 下一会话或下一个人靠这段接手。
