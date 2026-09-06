---
name: git-worktree-finish
description: "Explicit invocation ONLY: run /git-worktree-finish or ask for git-worktree-finish by name; never auto-trigger on 收尾 / worktree 清理 mentions. Runs git-finish in each non-main worktree, then lists all worktrees for the user to pick which to remove."
---

# Git Worktree 批量收尾（git-worktree-finish）

对仓库里除主 worktree 外的每个 worktree 依次执行 git-finish 收尾（提交 → 集成 → 验收 → 推送），全部结束后列出所有 worktree，让用户挑选要删除的。**仅显式调用激活**（`/git-worktree-finish` 或点名 git-worktree-finish），不因 收尾 / 清理 worktree 等字样自动触发。用户调用本 skill 即视为对收尾与推送的显式授权（同 git-finish），范围以预览确认的清单为准。

收尾细节不在这里复制：逐个 worktree 按 git-finish 的 0–5 步执行（需要时加载 git-finish 技能看细节）。必须**串行** —— 前一个集成进 main，下一个 rebase 的基点才是最新的，不要并行处理。

## 0. 摸清现场

```bash
git fetch origin
git worktree list --porcelain   # 全部 worktree；第一条是主 worktree
git worktree prune -n           # 预览：有哪些失效记录（目录已被手删）可清
```

主 worktree（列表第一条，通常检出 main）是集成点：各分支集成进 main 的快进操作在检出 main 的 worktree 执行，它自身一般无需收尾。顺手看一眼 main 是否领先远端（主 worktree 里 `git rev-list --count origin/main..main`），领先则计划里加一条「推送 main」。

对每个非主 worktree 判断是否需要收尾：

```bash
git -C <路径> status --short                  # 有输出 = 有未提交改动（含未跟踪）
git -C <路径> rev-list --count main..<分支>   # 0 = 分支已并入 main
```

无未提交改动且分支已并入 main → 标「已就绪」，跳过收尾；其余进入收尾清单。分支栏为空（detached HEAD）的 worktree 默认跳过并标注，用户点名要收再单独处理。

## 1. 预览计划（必做）

动手前展示计划，等用户确认：

```text
主 worktree：D:\work\agent-kit（main，集成点，不动）
按顺序收尾：
  1. ../agent-kit-login   feat/login   2 个文件未提交，3 个提交待合并
  2. ../agent-kit-pay     feat/pay     干净，1 个提交待合并
  3. ../agent-kit-docs    feat/docs    已就绪，跳过
```

用户可剔除某项、调整顺序；未获确认不执行。仓库只有一个 worktree → 提示直接用 /git-finish，到此为止。

## 2. 逐个收尾

按确认的顺序，对每个 worktree 完整走 git-finish：提交（conventional commits，按逻辑拆分）→ rebase origin/main → 在检出 main 的 worktree `git merge --ff-only <分支>` 快进 main（保持线性历史）→ 重新验收（测试 / 构建门禁）→ 推送 → 一句话总结。

- 每完成一个记一行结果：完成（commit 概况）或失败（原因）。
- 某个 worktree 卡住（冲突解不掉、测试红修不好）→ 标「收尾失败」，跳到下一个，**不让一个卡点堵死整批**；失败的原地保留，最后统一报告。

## 3. 挑选删除

全部结束后重新 `git worktree list`，编号列出，让用户挑要删的（可多选：「1 3」、「全部」、「都不删」；支持交互多选的工具用多选提问）：

```text
收尾完成，要删除哪些 worktree？（回复编号即可）
  [1] ../agent-kit-login   feat/login   已收尾，已推送
  [2] ../agent-kit-pay     feat/pay     已收尾，已推送
  [3] ../agent-kit-docs    feat/docs    收尾失败：测试红
  [4] D:\work\agent-kit    main         主 worktree
```

- 默认建议：已收尾的可删；收尾失败与主 worktree 保留。
- 只删用户选中的：`git worktree remove <路径>`。刚收尾完应是干净状态；git 以未提交改动 / 锁定为由拒绝 → 如实报告并问用户，明确说丢弃才 `--force`，否则保留。
- 删完问一句：已并入 main 的分支要不要顺手 `git branch -d` 清理（`-d` 只删已合并的，安全）；未合并的分支不动。
- 列表里有失效（prunable）条目 → `git worktree prune` 清掉记录。

## 4. 总结

```text
批量收尾完成：<共 N 个 worktree，收尾 M 个，删除 K 个>。
- <worktree 名>：<一句话结果>，已删除
- <worktree 名>：收尾失败（原因），已保留
- <worktree 名>：已就绪，未收尾
- 验证：<各 worktree 门禁结果；什么没验到，为什么>
```
