# Tasks: add-state-store

依赖前置:`add-core-session-channel` 的 `core_channel` 模块(通道、鉴权、订阅)已落地。
明确决策:不做旧 JSON 数据迁移,新库从零建表。

## 1. 状态库骨架

- [ ] 1.1 引入 `rusqlite`(bundled)依赖到 core 侧 crate;建 `sebas-state` 模块骨架(连接打开、WAL 模式、busy_timeout、user_version 读写封装)。验证:单元测试对空文件建库后读到 version 0,且 `PRAGMA journal_mode` 返回 wal。
- [ ] 1.2 实现专职写者 task:专用线程 + mpsc mailbox + oneshot 应答的 async 句柄,命令序列化执行。验证:并发 100 个写入命令全部按序提交,句柄 drop 后写者线程退出。

## 2. 自动迁移框架

- [ ] 2.1 迁移链执行器:启动时比对 user_version,升序应用迁移,每个迁移单事务含版本提升;失败整体回滚并中止启动。验证:测试用两个测试迁移,人为令第二个失败,断言库内容与版本号停在第一档。
- [ ] 2.2 高版本拒绝:库版本 > 二进制已知版本时返回带版本号的诊断错误,不修改文件。验证:手工把 user_version 改大,启动路径返回明确错误,文件 mtime/内容不变。
- [ ] 2.3 迁移前 VACUUM INTO 备份:`sebas.db.backup-<from>-<to>`,保留最近一份,旧的删除。验证:跑一次迁移后备份文件存在,恢复备份后库版本回到源版本。

## 3. 表结构与仓储

- [ ] 3.1 建表迁移:providers(软删标记)、model_aliases、settings、projects、sessions 表 + 索引;迁移 1 = 纯建表(无历史数据)。验证:空库跑迁移 1 后 schema 断言(表/列/默认值齐全),版本号为 1。
- [ ] 3.2 仓储层类型化方法:providers/aliases/settings/projects/sessions 的 CRUD,单事务多步操作(如删 provider 同时软删 + 清默认选择),全部显式列名。验证:仓储单元测试覆盖每个方法与原子多步场景。

## 4. 通道状态方法

- [ ] 4.1 `core_channel` 扩展 `state.*` 方法:快照查询 + CRUD 变更,鉴权复用通道既有校验。验证:集成测试——合法 peer 全方法往返;未鉴权 peer 收到拒绝。
- [ ] 4.2 提交后变更通知:带 scope 标签的事件流,允许多提交合并为一个通知。验证:订阅者断言收到 scope 正确的通知,且 10 连发合并 ≤3 次。

## 5. 消费端切换

- [ ] 5.1 router 侧退役 state_store.rs:读写改进程内状态库句柄;mode 修复逻辑(load repair)保留并指向新库。验证:现有 router 测试套件改测试夹具后全绿;删除 state_store.rs 后编译通过。
- [ ] 5.2 webui 切换:settings/provider 页改走状态方法;不可达时呈现"core 未连接 + 成因"、禁用变更入口。验证:手动 kill core 后页面显示降级态,恢复后自动回到正常渲染。
- [ ] 5.3 gateway 切换:文件热重载退役,providers/aliases 投影改订阅维护;admin API 后端改通道代理;断连时保持最后有效配置并在 /admin/stats 暴露不可用。验证:卡片改 provider → gateway 无重启生效;杀 core → 路由继续、stats 报不可用。

## 6. 收尾

- [ ] 6.1 端到端持久化测试:变更返回后立即 SIGKILL core,重启后状态在。验证:`cargo test` e2e 用例通过。
- [ ] 6.2 清理与文档:删除 providers.json/state.json/settings.json 的读写残留代码(旧文件留在磁盘、不再读取);模块文档写明"只加不改"迁移纪律与回滚路径。验证:grep 确认无旧文件读取路径残留;`cargo build` 无新增 warning。
