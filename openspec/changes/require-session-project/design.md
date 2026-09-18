## Context

会话的项目归属分布在四层：wire（`POST /api/sessions` 的 `project_id`）、内存映射（`Mapping.project_dir` + `pending_kind/model/mode`）、持久化（`state.json` 的 `MappingDto`）、呈现（rail 按 `project_id_for_session(info)` 分组）。不变量要成立，四层都必须收口——任何一层留一个「可以没有项目」的口子，幽灵会话就会从那里长出来。

## Goals / Non-Goals

- Goals：wire 层拒绝、内存层不丢、持久化层清退、呈现层有行；飞书例外明确且只有一条。
- Non-Goals：存量数据的归属猜测与迁移（直接删）；远端节点会话语义；rail 新增分组。

## Decisions

### D1：闸门放在 API 层，判据是「已注册项目」，不是「有路径」

`CreateSessionRequest.project_id` 保持 `Option<String>` 但语义改为必填：省略 / `null` / trim 后空串 → 400「project_id 必填：会话必须从属于项目」；未知 id → 400「未知 project_id: {id}」。**不**新增「直接给路径」的旁路——项目注册是唯一的目录入口（`path` 从不是 wire 标识），否则又会造出「有目录但没项目」的会话，与不变量同构地坏。

**Alternatives**：让 `project_id` 直接是 `String` 让 serde 拒绝缺失字段。否决——那会退化成 422 的框架缺字段文案，而我们要的是点名 `project_id` 的 typed 400（与 `agent` 必填的既有姿态一致）。

**Consequences**：wire 破坏性变更（旧调用方省略 `project_id` 一律 400）。同 binary 发布，不留兼容层——与 `desired_mode` 必填同一姿态。

### D2：spawn 失败只翻状态，不重建映射

`Map::fail_spawn` 此前 `Mapping::spawn_failed(reason)` 造一个全新映射整体替换，`project_dir` / `pending_kind` / `pending_model` / `pending_mode` / `desired_mode` 全部归零。后果有两重：会话从项目里消失（`project_id_for_session` 读到 `None`），agent 展示名也回退通用标签——把「启动失败」谎报成「这个会话没有归属、不知道谁在服务它」。

改为就地改状态：取 `get_mut`，只写 `state` 与 `last_active_unix`。构造器 `Mapping::spawn_failed` 随之删除（留着就是给后来者一个「造无项目失败会话」的陷阱）。

**Consequences**：失败会话在 rail 里带 `failed` 圆点（raw status 仍 `spawn-failed`）；`errors` 旅程的强断言（失败会话仍挂在项目行下）随之成立。

### D3：持久化双向清退，飞书是唯一例外；归档记录另论

- 判据收敛到一个函数 `mapping_may_lack_project(channel, project_dir)`：`channel == "feishu"` 放行；其余要求 `project_dir` 非空。
- `restore_json` 丢弃不合判据的存量行（warn 留痕）——操作者拍板历史数据可整片删，故不迁移、不猜归属。
- `dump_json` 同样跳过不合判据的内存行（warn）——双向兜底，保证幽灵行绝不被写回盘上喂给下一轮 restore。
- **内部归档记录键**（`closed-<hash>`，acp-session-mapping D4 的「原映射保留在存储，旧会话仍可被未来 load 寻址」）不是会话，不受判据约束；且 `preserve_closed_mapping` 顺手把**源映射的身份**（`project_dir` / kind / model / mode）抄进归档记录——否则归档记录自己就是「无项目行」，会被 D3 的清退顺手抹掉，恰恰废掉 D4 的承诺。

**Alternatives**：把归档记录也纳入判据、缺项目就丢。否决——那是用新不变量去拆旧承诺，且失败的是存储可寻址性，不是归属。

### D4：前端收口到「项目已经是前提」

创建入口本就只有 rail 项目行的「+」，但两处类型仍允许空：`NewSessionDialog.projectId: string | null`、`api.createSession({ projectId?: string | null })`。收紧为必填，并把弹窗的确认门禁从「agent 必选」扩为「agent 必选 **且** 有目标项目」。

`/sessions` 视图的创建表单是唯一真正的旁路（`api.createSession({ prompt, agent })`，没有项目）——补一个必选项目下拉：无项目可选时表单禁用 + 文案说明，有项目时默认选第一个；提交前本地先拦（给出可操作说明）而不是发一个注定 400 的请求。

**Consequences**：`/sessions` 的创建表单多一个下拉；空注册表时该表单不可用（这正是真相：没有项目就没有可归属的目标）。

## Risks / Trade-offs

- **破坏性**：旧 API 调用方（脚本/外部集成）省略 `project_id` 会开始 400。收益是没有归属的会话不可能再被造出来；操作者已就此拍板。
- **存量清退不可逆**：无项目历史会话在下一次 restore 时消失。操作者明确授权（「历史数据可以全部删除」），且这类会话本就没有可见面。
- **测试面大量改动**：约 30 处创建站点要带项目。用 `tests/support::scene_project_id` 之类幂等助手收敛，而不是每处手写注册。
