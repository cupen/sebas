# QA round13 验收简报（subagent 必读）

## 环境事故记录（2026-10-04，两起）

**事故一（22:05–22:35）**：首次起服漏设 `SEBAS_AGENT_ROUTER_URL`/`SEBAS_AGENT_MODEL`，native 渠道
不可达；期间外部 sbtestsuite 装配（非本流程启动）两次按映像名清场杀掉沙箱进程。

**事故二（22:34–22:49，重要）**：22:34 的恢复重启因 bash 双引号吞掉 `$env:` 前缀，core 在
**SEBAS_HOME 未设**状态下启动，落到了操作员真实 `C:\Users\cupen\.sebas`（core2.log 已取证）。
QA-B 后半段（~22:35–22:49）的 GUI 写操作（场景 agents：empty/error/slash/slow/thinking、
fake provider、默认模型设置、数条会话与消息）落入了真实 settings.db / projects.db。
config 显式钉死的路径（channel/acp sessions/work_dir/skills/media）未受影响。真实 home 的
清理属写操作，按红线留给操作员（精确清单见 REPORT.md）。

**当前环境（22:55 起生效，QA-A 用这套）**：
- 全新沙箱 `target/qa-r13-sandbox/`（DB 全新，账号已重建：admin/admin root、member/member、viewer/viewer）
- 二进制已改名 **`sebas-qa13.exe` / `fake-claude-qa13.exe`**（外部清场按 `sebas.exe` 映像名匹配，打不中了）
- core env 齐：`SEBAS_HOME` + `SEBAS_AGENT_ROUTER_URL` + `SEBAS_AGENT_MODEL=test` + `SEBAS_AGENT_PROVIDER_API_KEY`，**native 已 ok:true**（/api/summary 实测）
- WebUI `http://127.0.0.1:9877/`，router 8791
- 历史小坑：QA-B 早前把 native 形态用 acp 场景 agent（thinking/empty/error/slow）覆盖过（全部 PASS，见 findings-b）；QA-A 请优先走**真 native 渠道**（模型选 test/*）复核 thinking/tool-use/流式三形态，其余场景可仍用 acp 场景 agent

**QA-A 附加任务（QA-B 被中断的 8 项 + 复核 2 项，详见 findings-b.md 末尾）**：
auto 档独立复测、权限模式切换 F5 保持、parallel 双卡批准完成态、crash + token 续增
（R12-B-1）、crash 恢复假回执形态（O-2 只记录）、F5 历史保持、历史页驻留 + API 建会话
回合完成 info 瞬时条（R12-B-2）、深链直达 + 死会话 404 有界性（O-5 只记录形态）、
登出后登录页零 console 错误；B-1（allow 档 `perm` 无审批卡）复核 2 次采样、
B-2（thinking 面板留白）复采 1 次。

## 被测环境

- WebUI: `http://127.0.0.1:9877/`（auth 开）
- 账号：`admin/admin`（root）、`member/member`（member）、`viewer/viewer`（viewer）
- 沙箱根：`D:/workbench/repos-ai/sebas/target/qa-r13-sandbox/`（一切落点钉在里面；勿碰操作员真实 `~/.sebas`，真实实例端口 9797 禁触）
- ACP agent：fake-claude 桩（driver=claude）；native 渠道经 debug router（127.0.0.1:8791）的 `test` 场景模型
- 截图目录：`openspec/changes/fix-webui-qa-round13/verification/shots/`（PNG 直接写这里，文件名前缀见各簇分工）
- findings 写入：QA-A → `openspec/changes/fix-webui-qa-round13/verification/findings-a.md`；QA-B → `findings-b.md`（**边测边写盘，勿只在内存里攒**）

## fake-claude 触发词（按原文匹配消息文本）

- `perm`：权限环。**Auto 档（bypassPermissions）按桩设计走直接执行分支——tool_result 后没有「perm turn finished」收尾正文**，以「✓ tool_result + Done 终态」为完整预期，勿记缺陷（round10/11 已核实）。~~ask/edit/allow 档才出审批卡~~ **【round13 收口更正】ask/edit 档才出审批卡；allow 与 auto 同为放行档（round2 M-C6 裁决，`mode-vocabulary.ts` 明文：同映射 bypass tier、留审计），不出卡无环后正文是既定设计——QA-B 的 B-1 据此改判非缺陷**，批准后有环后正文
- `parallel`：一回合并行多个 tool_use（各自独立审批卡）
- `drip` / `stream`：流式滴流下发（测流式呈现、流式中「停止回复」）
- `flood`：大量文本（背压/渲染）
- `table`：表格渲染
- `crash`：agent 子进程崩溃 → 恢复链路（会话头部 token 累计、模型重报回执）
- `refuse`：非终结错误路径

## native `test/*` 场景（经 router 场景模型；会话创建时选对应模型）

- `test`：固定文案 + 回显，usage 全零
- `test/text`：单 text 块；`test/long`：>1000 字符按 32 字符窗口滴流
- `test/thinking`：thinking 块 + 正文（块序 thinking → text）
- `test/tool-use`：单工具环（mkdir -p .sebas-probe → tool_result → 终文本）
- `test/tools-parallel`：并行审批卡
- `test/full`：thinking + text + tool_use 混排
- `test/empty`：零 content 块、end_turn、usage 全零
- `test/error`：HTTP 5xx + api_error（失败呈现，会话仍可用）

## round12 在册项（已修/在册，**不要作为新缺陷重复上报**；可顺带验证形态）

1. R12-B-2 回合终点通知（tasks 1.x 已修）：历史页驻留 + 另一会话回合完成 → info 瞬时条；聚焦会话不弹是既定语义
2. R12-B-1 crash 后 token 累计回退（tasks 2.x 已修）：crash→恢复 累计应续增不回退
3. R12-A-1 技能「同步」对 member/viewer 应隐藏（tasks 3.x 已修）
4. O-2 crash 恢复后「模型已切换：default → fake」假回执（tasks 4.x **未修**，在册）
5. O-3 fakeacp 切模型无回执（tasks 4.x **未修**，在册）
6. O-5 死会话深链首载集中 404（tasks 5.x **未修**，在册）
7. round11 已核实：Auto 档 `perm` 无收尾正文 = stub 设计；N-1（workbench 直建会话不进 session_map、重启蒸发）在册待修

## 上报格式（findings-*.md，一缺陷一节）

```
## <ID>（P1|P2|P3）：<一句话标题>
- 现象：（看到的 vs 预期）
- 复现步骤：（GUI 操作序列，含账号/入口）
- 证据：shots/<文件名>.png（+ console 错误 / 网络请求要点）
- 初判：（前端渲染 / API 契约 / 事件流 / 持久化 / 文案，能定位到文件更好）
```

严重级：P1=功能不可用/数据丢失/权限越权；P2=主链路明显破损但有绕行；P3=打磨/文案/一致性问题。
另设「N-」前缀给「建议另立 change 的大工程」。**每个缺陷必须带可复现步骤**，不确定的标注「偶发」并写清采样次数。
