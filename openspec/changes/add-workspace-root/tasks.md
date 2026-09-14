## 1. 工作区根原语与控制平面装配

- [x] 1.1 `src/config.rs`：顶层 `Config` 增 `[workspace] root`；删除 `WatchdogWebUiConfig.allowed_roots` 字段与 `webui_allowed_roots()`。验证：单测覆盖 env 优先 / 配置次之 / 双缺省回退 cwd 三态解析，含 `allowed_roots` 旧键的配置文件解析不报错
- [x] 1.2 `sebas-webui/src/fs.rs`：`within_allowed_roots` 改形为 `within_workspace_root(candidate, root)`（canonicalize 两侧 + 逐分量前缀 + fail-closed）。验证：单测覆盖 symlink 逃逸、`..` 穿越、候选不可解析、root 不可解析时一切越界
- [x] 1.3 `sebas-webui/src/server.rs`：`WebUiState` 的 `allowed_roots` + `work_root` 合并为 `workspace_root: PathBuf`；builder 家族（`build_router_with_allowed_roots` 等）换签名；`safe_path` / `browse_dirs` 参数换根且 browse 起点为 workspace root。验证：改写后的 fs/server 既有测试全绿
- [x] 1.4 装配点 `src/webui_cmd.rs` 与 `src/run.rs`：按 env > config > cwd 回退计算 workspace root，回退时 `warn!` 告警（含回退路径与显式配置建议）。验证：单测/沙箱日志断言三态取值与告警文案

## 2. 本机项目面执法

- [x] 2.1 `sebas-webui/src/api.rs` `projects_add` 本地分支：越界 → 400「路径超出允许范围」，范围判定先行于存在性判定，错误不回显服务端解析路径。验证：api 层测试（界内注册成功 / 界外 400 / 不存在且越界同文案）
- [x] 2.2 `projects_list`：过滤越界本机项目（存储 canonical 路径与 canonical root 前缀比较；root 解析失败全部隐藏 + warn）。验证：测试覆盖界内可见 / 界外隐藏 / root 缺失全隐藏
- [x] 2.3 会话面拒绝：detail / message / switch / 携越界项目的 create 返回 4xx typed 拒绝；close / archive 放行；`projects_branch` 对越界项目按不可达处理。验证：api 层测试逐端点断言
- [x] 2.4 远端注册 `projects_add_remote`：`within_workspace == false` 拒绝，字段缺省（老节点应答）放行。验证：backend 桩测试覆盖拒绝、放行、缺省三态

## 3. 节点侧与协议

- [x] 3.1 `sebas-node-link/src/lib.rs`：`SessionResult::PathChecked` 增 `within_workspace: bool`（serde `#[serde(default = "default_true")]`）。验证：帧编解码往返测试 + 旧应答 JSON（无该字段）反序列化为 true
- [x] 3.2 `sebas-node/src/{config.rs,session.rs}`：`NodeConfig.workspace_root` 字段 + `SEBAS_WORKSPACE_ROOT` env 读取 + cwd 回退启动告警；`CheckPath` handler 增 containment 判定。验证：节点侧测试（界内 `(exists,is_dir,within)`、越界 within=false、未配置回退告警）
- [x] 3.3 协议贯通验证：主控经 `check_node_path` 拿到 within 判定并流入 2.4 的拒绝逻辑。验证：node_link projection / core_channel 测试贯通

## 4. Harness 与文档

- [x] 4.1 `tasks.py`：webui-sandbox / webui-server / e2e / acceptance 沙箱 config 钉 `[workspace] root`（或 env）指向沙箱目录。验证：`invoke testsuite-webui-sandbox` 起服后注册沙箱内项目成功、无回退告警
- [x] 4.2 `AGENTS.md` 沙箱菜谱与 README/部署文档：补 `[workspace] root` 与 `SEBAS_WORKSPACE_ROOT` 说明、allowed_roots 迁移（多根→符号链接合并）、cwd 回退告警解释。验证：文档评审（交付物）
- [x] 4.3 e2e/验收套件补越界场景：注册越界 400、越界历史项目列表隐藏、打开/发消息拒绝、close/archive 放行。验证：`invoke testsuite-e2e`（新增用例）通过

## 5. 收尾验证

- [x] 5.1 全量质量门：`rtk cargo test`（workspace）与 `rtk cargo clippy` 全绿。验证：命令退出码 0
- [x] 5.2 沙箱端到端冒烟：按 AGENTS.md 菜谱起双进程沙箱，验证告警三态、注册/列表/打开四处执法、browse-dirs 起点与越界 root 拒绝；`GET /api/summary` 正常。验证：冒烟记录（curl 输出）符合 spec 场景
