# Design: simplify-mode-menus

## Context

词汇面现状（事实，已侦察核实）：

- `mode-vocabulary.ts` 是权限模式词汇的单一出处：`MODE_OPTIONS` 四项已是 Title Case（Ask/Edit/Allow/Auto，add-agent-settings-and-session-titles 7.2 落地）；同文件的 `MODE_DEFAULT_LABEL = '默认（Ask）'` 被两个菜单的空值首项消费。
- 创建弹窗 `new-session-dialog.ts:377` 与 composer `workbench-composer.ts:933` 各渲染 `<wa-option value="">${MODE_DEFAULT_LABEL}</wa-option>`。弹窗预选 `ask`，空值项非预选但可点；composer 侧空值项注释自述「只作旧会话展示兜底，选中即不切换」。
- 后端 `sebas-webui/src/api.rs` 的 `valid_session_mode("")` 为假——`create_session` 对 `mode: ""` 返回 400（「mode 非法」）。弹窗的 `mode: e.detail.mode` 原样透传（`project-rail.ts` confirmNewSession），空值路径一旦被用户点中即创建失败。
- 状态库 schema `desired_mode TEXT NOT NULL`（`src/sebas_state/repo.rs:147`）+ 服务端缺省 `ask`：不存在 mode 为空的存量会话，composer 的「旧会话兜底」没有真实对象。
- 主 spec `agent-workbench` 已是目标态：composer mode 切换「option labels SHALL be Title Case (Ask/Edit/Allow/Auto)」「SHALL NOT render an empty or placeholder mode state」；创建弹窗「SHALL carry a permission-mode choice (`ask | edit | allow | auto`) prefilled with `ask`」。实现落后于 spec，本 change 是收敛而非新政策。

流程注记：grilling 问询未获应答，按推荐口径（菜单残留中文收敛）自主推进；各分叉取舍以主 spec 既有文字为准绳——spec 是双方最强的共识锚点，本设计不逆着任何既有 SHALL 行事。**评审修正（用户拍板）**：模式描述不得内联进菜单项标签（那是 7.2 之前「ask（逐次询问）」式漂移的根源），但也不许就此丢掉——须以别的方式提示。组件事实（webawesome 3.12 参考文档）：`wa-option` 槽位仅 default（标签）与 `end`（图标），**没有 per-option 描述槽位**；`wa-select` 有 `hint` 属性/槽位（控件下方描述行）；`title` 全局属性在自定义元素上照常出原生悬浮提示。

## Goals / Non-Goals

**Goals:**

- 两个 mode 菜单的选项集收敛为恰四词 Ask/Edit/Allow/Auto，无空值项、无中文注解。
- 模式解释以**非内联通道**保留：悬浮 title + 创建弹窗动态 hint 行，措辞收敛进词汇源文件（`ModeOption.description`）。
- 消灭「弹窗选默认 → `mode: ""` → 400」的潜在故障路径。
- 词汇出处保持单一：改动后 `MODE_OPTIONS` 仍是唯一来源，`MODE_DEFAULT_LABEL` 退役删除。

**Non-Goals:**

- 不改 wire 词汇与后端校验（`ask|edit|allow|auto`、`valid_session_mode` 原样）。
- 不动 dashboard 状态章的中文措辞（`modeBadgeLabel`：逐次询问/自动接受编辑/放行/自动执行）——那是中文 UI 的正文文案，polish-workbench-walkthrough-ux 4.2 的有意设计，不属于「菜单注解」。
- 不把本仓源码的中文注释改英文——「去掉中文注释」取「菜单项的中文注解」义，不取「代码注释」义；全仓注释语言为中文，单独改一个文件反而制造不一致。
- 不处理「未记录 mode 的展示兜底」之外的 composer mode 面行为（modeEditable、切换在途态等均不动）。

## Decisions

- **D1 删除空值占位项，而非改词保留。** 备选：把「默认（Ask）」改成英文 `Default` 留在菜单里。否因：主 spec 明令 composer 不得渲染空/占位 mode 态、弹窗选项集恰为四词；且 `Default` 与 `Ask` 在 wire 上语义重复（服务端缺省即 `ask`），留一个第五项违背「保持简洁」的诉求本身。删除后 composer 的 `@change` 里 `if (v)` 空值守卫退化为死分支，一并摘除。
- **D2 `MODE_DEFAULT_LABEL` 整体删除，不留别名。** 两个消费点都随空值项消亡，保留导出常量即保留第二词汇源的 temptation；测试改为直接断言 `MODE_OPTIONS` 形状。
- **D3 测试断言从「含默认项」翻转为「恰四词」。** `new-session-dialog.test.ts` 旧断言（`options[0].label === '默认（Ask）'` 及全序列）改为断言选项集恰为 Ask/Edit/Allow/Auto 且无空值 `value=""` 项；`workbench-composer.test.ts` 的 `labels[0]` 断言改为 `'Ask'`；Playwright `mode.spec.ts` 的 `toContainText('默认（Ask）')` 改为断言四词齐全且不含「默认」。wire 值断言（小写 ask/edit/allow/auto）保持不动。
- **D4 弹窗确认路径不加防御性改写。** 备选：rail 侧 `mode: e.detail.mode || 'ask'` 兜底。否因：占位项删除后空值源头已不存在，加兜底是给不存在的路径上保险，还会掩盖未来回归；让类型与 UI 结构保证正确（`this.mode: string` 恒为四词之一）比运行时补丁可靠。
- **D5 描述走「title 悬浮 + 弹窗动态 hint 行」双通道，措辞进词汇源。** `ModeOption` 增加 `description: string`（恢复 7.2 移除的解释语义：逐次询问 / 自动接受编辑 / 放行并留审计 / 自动执行（不门控，留审计）），与 `MODE_OPTIONS` 同处一个文件——菜单裸词与中文解释在源码里相邻定义，展示层永不拼接。两个菜单的 `wa-option` 挂 `title=${m.description}`（原生悬浮，零布局影响）；创建弹窗的 `wa-select` 加 `hint` 行，绑定当前选中模式的描述（弹窗有纵向空间，选中即可见，键盘/触屏用户也有可读通道）。否掉的备选：① 描述拼回选项标签（用户明令禁止，正是 7.2 要消除的形态）；② per-option 富描述（wa-option 无描述槽位，组件层不存在）；③ composer 也加 hint 行（工具栏受 spec 938 紧凑宽度帽与左右基线网格约束，加行即破坏；composer 所在会话的当前模式另有 dashboard 徽章中文措辞可读，悬浮 title 够用）。
- **D6 描述措辞与 `modeBadgeLabel` 分开维护，不互相调用。** 备选：`description` 直接复用 `modeBadgeLabel(value)`。否因：徽章是状态章，措辞求短（「放行」）；菜单悬浮求解释（「放行并留审计」）——两处修辞目标不同，强行合一会互相牵制。同文件相邻定义已保证口径可见性（同屏可审），漂移风险由「同一个词只在一个文件」约束兜住。

## Risks / Trade-offs

- **旧会话前端缓存**：极端情况下（长开的旧标签页）composer 收到的 `currentMode` 若为空字符串，select 将显示空而非「默认（Ask）」。可接受：schema NOT NULL + 服务端缺省 `ask` 下不存在这样的行，且刷新即自愈；为幻影对象保留兜底项正是本次要删的东西。
- **e2e 断言漂移**：`mode.spec.ts` 若还有其他对首项的隐式依赖（如 `selectOption('')`），任务里逐条核查该 spec 文件全量断言后再改，不只动 46–47 行。
- **title 悬浮通道的局限**：原生 title 提示有延迟、样式不可控、触屏设备基本不可用；composer 的键盘用户拿不到描述。接受：composer 空间约束下这是唯一零布局影响的通道，且它只是补充——会话当前模式的可读中文措辞由 dashboard 徽章承担；弹窗 hint 行是主要解释面，不受此限。
- **描述措辞双出口**：`description` 与 `modeBadgeLabel` 措辞并行（D6），存在理论上的一致性维护成本。缓解：同文件相邻定义 + 评审时同屏可见；任一侧改词都会在同一处看到另一侧。
- **回滚成本低**：纯前端展示面，git revert 即完整回滚，无数据/协议痕迹。
