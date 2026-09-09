# Tasks — clarify-driver-layer-boundary

## 1. 边界澄清（已完成）

- [x] 1.1 agent-driver MODIFIED「AgentDriver abstraction with two implementations」：补抽象/策略层边界 + 新场景「ACP driver delegates lifecycle to the runtime layer」；verify: delta 保留全部既有 SHALL 与场景
- [x] 1.2 acp-driver MODIFIED「One subprocess per session」：补运行时层定位 + 新场景「Runtime serves any driver kind」；verify: delta 保留既有 SHALL 与场景
- [x] 1.3 两侧主 spec Purpose 直改（openspec 约定：Purpose 不进 delta）；verify: 主 spec Purpose 含「抽象层/运行时层」表述
- [x] 1.4 glossary「执行体」ACP 桥补 agent-driver/acp-driver 分层，修正陈旧 feishu-bridge 引用；verify: glossary 措辞与边界一致

## 2. 归档

- [ ] 2.1 `openspec archive clarify-driver-layer-boundary -y`；verify: validate --specs 通过、主 spec 含澄清文本
- [ ] 2.2 `openspec validate --all` 无 ERROR；verify: invalid = 0
