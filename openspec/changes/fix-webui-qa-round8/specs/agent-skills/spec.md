## ADDED Requirements

### Requirement: 技能预览只渲染正文

技能详情预览 SHALL 只渲染 SKILL.md frontmatter 之后的正文内容，SHALL NOT 把 frontmatter 块当作正文渲染。

#### Scenario: 打开带 frontmatter 的技能预览
- **WHEN** 操作者点击技能名展开 SKILL.md 预览
- **THEN** 预览内容自正文开始，不含 `---` 包裹的元数据头块
