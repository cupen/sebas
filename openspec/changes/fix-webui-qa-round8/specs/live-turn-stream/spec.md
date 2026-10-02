## ADDED Requirements

### Requirement: 零输出判定在终态时点求值

空回合 notice 的判定 SHALL 在回合终态（含错误条目落地）之后进行；以错误收尾的回合 SHALL NOT 被附加「零输出」notice。

#### Scenario: 错误回合不误报空回合
- **WHEN** 一个回合以错误形态收尾（上游 5xx 等）
- **THEN** 转录只呈现错误形态，不出现「本回合无输出」类 notice

#### Scenario: 真空回合 notice 不回归
- **WHEN** 一个回合确实零内容块且正常收尾
- **THEN** 空回合 notice 按既有语义照常出现
