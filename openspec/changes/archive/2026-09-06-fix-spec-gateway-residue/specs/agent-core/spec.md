## MODIFIED Requirements

### Requirement: LLM channel

The system SHALL reach the LLM exclusively by speaking the Anthropic Messages streaming protocol to a configured endpoint authenticated with a configured credential. The endpoint SHALL be configurable: a provider endpoint used directly — the default path, requiring no router — or a router as an optional routing layer. The system SHALL NOT embed any provider SDK. Tool call arguments SHALL be assembled from incremental JSON fragments delivered by the stream before any tool executes.

#### Scenario: Tool arguments arrive as fragments

- **WHEN** a streamed response delivers a tool call's arguments as multiple incremental JSON fragments
- **THEN** no tool starts executing before the arguments assemble into valid JSON
- **AND** the executed tool receives the complete argument object

#### Scenario: Direct provider endpoint without a router

- **WHEN** the client is configured with a provider base URL and API credential directly
- **THEN** requests are sent to that endpoint and no router is contacted
