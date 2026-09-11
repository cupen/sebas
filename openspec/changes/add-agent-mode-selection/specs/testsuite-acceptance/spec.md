## ADDED Requirements

### Requirement: 远端节点 mode 旅程

验收套件 SHALL 包含远端节点会话的 mode 旅程用例（真 sebas-node 二进制、EchoBody 桩，零真模型调用）：

- **创建带 mode**：在远端节点上以 mode=allow 创建会话，断言投影的 desired_mode 为 allow 且节点回报一致；门控动作（`run:` 输入）不再进入 waiting 而直接执行（与既有旅程的 waiting 断言形成对照）。
- **中途切换**：对运行中的远端会话切换 mode（如 allow→ask），断言 SetMode 经链路生效、后续门控动作重新进入 waiting、投影 mode 更新。
- **离线拒绝**：节点离线时创建带 mode 会话按既有规则如实拒绝（点名节点），不建占位。

#### Scenario: 远端 allow 会话免门控

- **WHEN** 在远端节点以 mode=allow 创建会话并发送 `run: ls -la`
- **THEN** 动作直接执行（不产生 waiting 审批），会话回合完成

#### Scenario: 远端会话中途切回 ask 恢复门控

- **WHEN** 对该会话切换 mode 为 ask 后再发送 `run:` 输入
- **THEN** 动作被门控为 waiting 等待控制面决定，投影 mode 显示 ask

#### Scenario: 零 token 断言

- **WHEN** 该旅程全程运行
- **THEN** 不发生任何真实模型调用（节点 agent 为 EchoBody 桩）
