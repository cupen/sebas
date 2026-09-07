# Tasks: add-webui-allowed-roots

## 1. 配置与状态注入

- [x] 1.1 `src/config.rs`：`[watchdog.webui]` 增加 `allowed_roots: Vec<String>`（默认空，`~` 展开，非绝对路径告警），验证：`cargo build` 通过 + 单测覆盖展开/空列表语义
- [x] 1.2 `sebas-webui/src/server.rs`：`WebUiState` 增加 `allowed_roots` 字段；`src/run.rs` 与 `src/webui_cmd.rs` 两处接线注入（默认根自动并入白名单），验证：`cargo build` + 现有 webui 测试全绿

## 2. 范围判定核心

- [x] 2.1 `sebas-webui/src/fs.rs`：`safe_path` 增加白名单参数——显式 `root` canonicalize 后必须位于某 allowed root 之下，否则「路径超出允许范围」错误；空白名单跳过校验；验证：新增单测（命中 / 越界 / 空白名单 / symlink 边界 / 默认根不受影响），`cargo test -p sebas-webui fs::`
- [x] 2.2 提取共享判定函数（路径是否位于白名单内）供注册校验复用，验证：单测直接覆盖该函数

## 3. 项目注册范围校验

- [x] 3.1 `POST /api/projects` handler 在 `projects::add` 前做白名单判定（fail-closed：越界或无法解析 → 400，不泄露解析后路径），验证：handler 层单测覆盖越界/命中/空白名单三态

## 4. 验证与收尾

- [x] 4.1 全量回归：`cargo test` + `cargo clippy`，验证无回归
- [x] 4.2 沙箱 e2e 冒烟（`invoke testsuite-webui-sandbox`）：配置 `allowed_roots` 后 browse-dirs 带 `root=<越界>` 返回 400、`POST /api/projects` 越界 400、白名单内正常；未配置时行为与现状一致，验证：记录 curl 输出
  - 补充：冒烟抓到接线层回归——默认根曾被无条件并入白名单，导致未配置时强制生效；已提取纯函数 `config::webui_allowed_roots`（空配置 = 空表不启用）并补单测，六项 curl 场景全过后沙箱已清理
- [x] 4.3 更新 `openspec/specs/webui/spec.md` 的准备：确认 delta 场景与实现一致（archive 阶段执行），验证：`openspec validate add-webui-allowed-roots --strict` 通过

## 5. 错误处理与降级（design D6）

- [x] 5.1 `api/ws.ts` / `api/shared-ws.ts`：暴露连接状态（connected / reconnecting）；`app-shell.ts` 渲染全局断线横幅，恢复即消失（重连沿用现有 `sebas:refetch` 刷新），验证：单测模拟断线→横幅出现、恢复→消失
- [x] 5.2 `api/client.ts`：fetch 抛出的网络级失败（TypeError，无 HTTP 响应）包装为可识别的「服务不可达」错误类，`ApiError` 语义不变，验证：client 单测覆盖两类错误可区分
- [x] 5.3 `views/dashboard.ts` / `views/project-rail.ts` / `views/sessions.ts`：初始加载失败显示内联失败态 + 重试入口而非空白，验证：各视图单测
- [x] 5.4 `views/workbench-composer.ts`：summary 轮询本身失败时等同 `reachability.ok = false`（禁用提交门 + 不可达提示），验证：composer 单测
- [x] 5.5 沙箱冒烟：`invoke testsuite-webui-sandbox` 起服务后 kill 进程，观察横幅出现与刷新后自动恢复，验证：记录行为输出
