---
name: git-worktree
description: "Explicit invocation ONLY: run /git-worktree or ask for git-worktree by name; never auto-trigger on worktree / 并行开发 mentions. Args: 功能名 then 分支名. Creates ../<repo>-<功能名>, reuses the branch if it exists else forks it, previews before creating."
---

# Git Worktree 开工区（git-worktree）

从当前 git 仓库在旁边目录创建 worktree 并检出指定分支，与当前工作区互不干扰。**仅显式调用激活**（`/git-worktree` 或点名 git-worktree），不因对话里出现 worktree / 并行开发字样自动触发。

两个必填入参，按顺序：

1. **功能名** — 目录后缀，只决定目录叫什么，与分支名不必对应；
2. **分支名** — worktree 里检出的分支，已存在则复用，不存在则 fork。

目标路径：`../<项目目录名>-<功能名>`（与仓库同级）。

## 0. 摸清现场

```bash
git rev-parse --is-inside-work-tree    # 不是 git 仓库 → 告知用户并停止
git rev-parse --show-toplevel          # 仓库根；basename 即「项目目录名」
git branch --show-current              # 当前分支（fork 的默认基点）
git worktree list                      # 已有 worktree 及其占用的分支
git branch --list --all <分支名>        # 入参分支是否已存在
```

## 1. 定路径与模式

- 路径 = `<仓库根>/../<项目目录名>-<功能名>`；功能名中的 `/` 等路径非法字符替换为 `-`（`login/2` → `myrepo-login-2`）。
- 分支已存在（仅存于远端时 git 会自动建同名本地分支跟踪）→ 复用：`git worktree add <路径> <分支>`。
- 分支不存在 → fork：`git worktree add -b <分支> <路径>`，基点为当前分支 HEAD；用户另指了基点（main、某 commit）则用之。
- 入参缺失就只问缺的那项，不猜；用户在对话里已明确给出的取值直接采用。
- 冲突预案（任一命中先解决再进预览）：
  - 目标路径已存在且非空 → 换功能名或让用户清理，绝不覆盖。
  - 分支已被其它 worktree 检出 → git 会拒绝二次检出；换分支或换功能名 fork 新分支，不用 `--force` 硬闯。

## 2. 预览确认（必做）

执行写操作前展示预览并等用户明确同意：

```text
仓库:   agent-kit（D:\work\agent-kit，当前在 main）
模式:   fork 分支 feat/login（基点 main @ a1b2c3d）   # 复用则为：复用分支 feat/login（本地）
路径:   ../agent-kit-login（不存在，可用）                # 功能名 login，与分支名解耦
命令:   git worktree add -b feat/login ../agent-kit-login
```

用户要调整（换功能名 / 换分支 / 换基点）就改完再预览一次；未获确认不执行。

## 3. 创建与交付

```bash
git worktree add -b <分支> <路径> [<基点>]   # fork 模式
git worktree add <路径> <分支>               # 复用模式
```

- 成功 → 一句话交付：路径、分支、`cd` 过去即可开工；完工收尾交给 git-finish，多个 worktree 要批量收尾时用 git-worktree-finish。
- 失败 → 如实报告 git 原因（路径被占 / 分支冲突），按第 1 步预案调整后重新预览确认，不盲目重试。
