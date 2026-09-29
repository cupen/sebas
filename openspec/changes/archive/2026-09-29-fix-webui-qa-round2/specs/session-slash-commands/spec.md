## ADDED Requirements

### Requirement: Command submission has a transcript receipt and one dispatch path

提交受支持的 slash 命令 SHALL 在转写产生可见的回执条目（不得只有瞬态 toast）；命令的产出 SHALL 作为独立条目呈现，不得并入前一条 assistant 段落。composer 对以 `/` 开头的文本 SHALL 单一分派路径——经键盘提交与经发送按钮提交得到一致的命令/普通消息判定。

#### Scenario: compact leaves a receipt

- **WHEN** 操作员提交 /compact
- **THEN** 转写出现该命令提交的回执条目，命令产出（若有）作为独立条目呈现

#### Scenario: enter and button dispatch identically

- **WHEN** 同一段 slash 文本分别经键盘与发送按钮提交
- **THEN** 两次走同一分派路径（按命令或按普通消息的判定一致）

#### Scenario: unknown command feedback

- **WHEN** 操作员提交该会话 agent 不支持的 slash 命令
- **THEN** UI 给出可见的不支持反馈，输入内容保留
