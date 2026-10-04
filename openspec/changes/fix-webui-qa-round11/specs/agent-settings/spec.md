## MODIFIED Requirements

### Requirement: Deleting an agent clears project defaults

When an agent is deleted and a project's remembered default agent refers to it, that project default SHALL be cleared (creation falls back to the catalog's normal default selection) instead of leaving a dangling reference. The delete confirmation SHALL state the consequences in unambiguous copy: each clause SHALL read as a complete sentence with consistent spacing, so phrases like「在创建会话下拉中立即可见」cannot be misread across a line/word boundary (the confirmation names the agent, states that existing sessions run to natural completion, that referring project defaults are cleared, and that the agent disappears from the creation dropdown immediately).

#### Scenario: project default is cleared on delete

- **WHEN** the operator deletes agent `opencode` which is project P's remembered default agent
- **THEN** project P's default agent is cleared, and the create-session dialog for P preselects by the normal fallback rule

#### Scenario: confirmation copy is unambiguous

- **WHEN** the operator opens the delete confirmation for an agent
- **THEN** the copy reads as complete, unambiguous sentences with consistent spacing, with no clause that can be misparsed (e.g. no「下拉 中立即可见」break)
