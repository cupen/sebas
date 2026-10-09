# add-local-usage-statistics — Tasks

## 1. 中立域类型与共享聚合

- [x] 1.1 `sebas-domain` 新增 usage 域类型：回合用量记录（ts/model/provider/status/四类 token/latency/error）+ 时序请求/响应形状；`sebas-router` 的 `UsageRecord`/聚合请求改为 domain re-export，wire 逐字不变——跑 `cargo test -p sebas-router`（含协议 golden fixture）确认零漂移
- [x] 1.2 聚合纯函数下沉 domain（记录集 + granularity + days + tz_offset → 零填充桶/分模型/分 source 小计）；router 侧改调 domain 实现；单测断言「同输入同桶形」与既有 router 聚合结果逐字段一致——`cargo test -p sebas-router -p sebas-domain`

## 2. 本地账本（usage_local.db）

- [x] 2.1 sebas home 映射表扩充 `usage_local.db` 逻辑名（env `SEBAS_USAGE_LOCAL_DB` 覆盖）+ `src/sebas_state` 注册表 DDL（`LocalUsageRow` 行 struct 留 core 消费模块）；单测钉落点派生与 env 覆盖——`cargo test`（core 侧）
- [x] 2.2 core 侧写入器：`sebas-db` 单写 actor + 有界通道 + 批量提交，满则丢弃 + warn 绝不阻塞回合；保留期双闸（默认 30 天/20 万行/每小时）后台清理 + 计数日志；单测覆盖 sink 语义与清理——`cargo test`
- [x] 2.3 落账钩子：ACP 回合 `UsageUpdate` 结算路径逐回合落一行（ts=完成时刻、四类 token、未上报不冒充零）；单测用 fake 驱动断言一回合一行、重启后仍在——`cargo test`
- [x] 2.4 双算规避：native 注入 `SEBAS_AGENT_ROUTER_URL` 的会话回合不落本地行，ACP 会话恒落；单测钉两种装配的分叉——`cargo test`

## 3. native usage 解析

- [x] 3.1 native 内核解析上游响应 usage（Anthropic/OpenAI 两形状）→ 中立类型随回合事件上报 core；快照 `usage` 从恒 None 改为如实携带；单测覆盖两形状与无 usage 响应——`cargo test -p sebas-agent`
- [x] 3.2 native 回合（直连）落账接线：解析结果经回合结算写 `usage_local.db`；单测断言直连回合落行、无 usage 回合只计请求数——`cargo test`

## 4. 查询面双源化

- [x] 4.1 core `/api/usage/timeseries` 加 `source` 参数：router=既有反代语义（不可达结构化 cause）；local=本地聚合；all=本地聚合 + 尽力反代合并，router 不可达仍 200 且带 `router_cause`；非法 source 400；单测三分支 + 参数校验——`cargo test -p sebas-webui`
- [x] 4.2 桶内 per-source 小计进响应形状（domain 类型同步）；单测断言 all 合并不重算（本地行 + router 行同模型同桶 → 合计 = 两源之和）——`cargo test`

## 5. WebUI

- [x] 5.1 /usage 视图加来源切换（全部/router/本地，默认全部）；`source=all` 且带 `router_cause` 时图表照常渲染本地数据 + 局部「router 部分不可达」警示条（区别于整页不可达空态）；视图测试覆盖切换、警示条与既有空态——`pnpm test`（frontend）

## 6. 联调与回归

- [x] 6.1 沙箱全链路：`--debug` router + fake-claude 造 router 源数据，ACP 会话回合造本地源数据，native 直连回合造本地源数据——验证三口径聚合、all 合并无双算、停 router 后 local/all 照常出数；UI 三种来源切换与局部警示（复核发现的 ACP 落账行 token 丢失缺陷已修：driver 成功帧改为 usage 先行，回归用例 `real_driver_order_finished_before_usage_still_lands_reported_tokens` 转正）
- [x] 6.2 回归：`cargo test` 全绿 + 前端 `pnpm test` 全绿；`invoke testsuite-e2e` 不红
