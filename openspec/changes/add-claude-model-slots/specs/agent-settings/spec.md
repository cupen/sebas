## MODIFIED Requirements

### Requirement: Agents are managed from the Settings modal

The Settings modal SHALL expose an `Agents` section as a single unified catalog with one row per agent: the built-in `native` entry, every agents-store row, and every installable recipe entry (the auto-install catalog) appear in the same list — a separate recipes section SHALL NOT exist. Each row SHALL display the agent's install status (`内置` / `已安装` / `未安装`, with honest reachability detail for installed agents). A not-installed recipe row SHALL offer its install action; an installed row SHALL offer a gear customization action that opens the create/edit form; store rows remain deletable (native is neither installable nor deletable). The create/edit form SHALL offer driver shapes `claude` (binary path, default `claude`), `opencode` (ACP command prefilled), `pi`, and custom ACP (arbitrary command argv); stored definitions SHALL use the existing driver tags (`claude` | `acp` | `pi`). Deleting an agent SHALL be a confirmed action and SHALL NOT affect already-created sessions.

#### Scenario: unified list shows recipe rows with install status

- **WHEN** the Agents section renders while the `opencode` recipe is not yet installed and a `claude` store row exists
- **THEN** both appear as rows in one list, `opencode` showing `未安装` with an install button and `claude` showing `已安装` with a gear button, and no separate recipes block exists

#### Scenario: gear opens the customization form

- **WHEN** the operator clicks the gear on an installed agent's row
- **THEN** the create/edit form opens prefilled with the stored definition, including the model slot mapping for claude agents

#### Scenario: create an opencode agent without restart

- **WHEN** the operator adds an `opencode` agent via the form and saves
- **THEN** the new agent is selectable in the create-session dialog immediately, without restarting any process

#### Scenario: edit updates the catalog live

- **WHEN** the operator edits an agent's display name or launch command and saves
- **THEN** the catalog endpoint and the create-session dialog reflect the change without a restart

#### Scenario: delete removes the agent for new sessions only

- **WHEN** the operator deletes an agent that has an active session
- **THEN** the active session keeps working to the end of its lifecycle, and the agent disappears from the catalog and the create-session dialog

### Requirement: Agent form covers spawn-critical fields

agent 新建/编辑表单 SHALL 覆盖 spawn 所需字段：sessions 目录、work 目录与启动参数（键值形态），校验口径与 config 种子条目一致；用表单建出的 agent SHALL 能直接 spawn 可用会话，无需手改 config。编辑表单 SHALL 以存储值预填全部字段，包括 display name。表单 SHALL 按形态渲染差异化配置段：`claude` 形态展示模型档位映射段（见「Model slot mapping is a claude agent setting」），`opencode`/`pi`/custom 形态 SHALL NOT 渲染任何模型配置段。

#### Scenario: GUI-created agent is spawn-ready

- **WHEN** 操作员经设置表单以完整字段（含 sessions 目录、work 目录、参数）创建 agent 并用其开session
- **THEN** 会话以表单填写的目录与参数 spawn 成功并完成回合

#### Scenario: edit prefills display name

- **WHEN** 操作员打开已存有 display name 的 agent 的编辑表单
- **THEN** display name 字段回填存储值而非留空

#### Scenario: model slot section renders per shape

- **WHEN** 操作员分别以 `claude` 形态与 `opencode` 形态打开表单
- **THEN** `claude` 形态显示模型档位映射段，`opencode` 形态不显示任何模型配置段

## ADDED Requirements

### Requirement: Model slot mapping is a claude agent setting

A claude agent definition SHALL carry an optional model slot mapping: for each of the fixed builtin slots `fable`/`opus`/`sonnet`/`haiku` an optional model id picked from the model catalog (provider models and router aliases with equal standing), plus a default marker. A mapping SHALL mark exactly one enabled slot as default; a slot name outside the fixed four SHALL be rejected. The mapping SHALL persist in the agents store and take effect from the next spawn without restart, editable through both the Settings form and the config seed. The config.toml `[acp.agents.<id>]` models key SHALL accept only the structured mapping form — the legacy bare-list form SHALL be rejected at parse time naming the key — and a stored legacy list value SHALL be treated as no mapping.

#### Scenario: mapping saved via the form is live for the next spawn

- **WHEN** the operator saves a slot mapping through the gear form and creates a new session with that agent
- **THEN** the spawn uses the saved mapping without any process restart

#### Scenario: legacy bare list in config is rejected at parse time

- **WHEN** config.toml carries `[acp.agents.claude] models = ["opus", "sonnet"]`
- **THEN** startup rejects the config with an error naming the `models` key and the expected structured form

#### Scenario: invalid mapping is a typed validation error

- **WHEN** a mutation submits a mapping whose default marker sits on a disabled slot, or enables slots without marking any default
- **THEN** the mutation is rejected with a typed validation error naming the offending slot

#### Scenario: value picker offers the whole catalog

- **WHEN** the operator opens a slot's value picker in the claude form
- **THEN** the picker lists provider models and router aliases side by side, with no provider-mode-based filtering
