## ADDED Requirements

### Requirement: Model entries carry capability tags

A provider's model list SHALL be a list of entries rather than a list of bare strings.
Each entry SHALL carry the model id and that model's capability tags. `text` SHALL be
implicit for every entry and SHALL NOT be stored; `vision`, `audio`, and `video` SHALL be
explicit and selectable, marking a model that accepts image, audio, or video input
respectively. Tags SHALL be per entry and editable by the operator. Legacy stored lists of
bare strings SHALL be accepted on read and normalised to entries carrying `text` alone, so
existing data keeps working without an offline migration. Capability tags SHALL be
metadata: they SHALL NOT alter routing, protocol selection, or whether a request is
accepted.

#### Scenario: an entry carries its id and tags

- **WHEN** a provider's model list is read
- **THEN** each element is an entry exposing its model id and its capability tags, with
  `text` implied on every entry

#### Scenario: multimodal tags are stored explicitly

- **WHEN** the operator marks a model as accepting image input
- **THEN** that entry carries `vision` in addition to the implicit `text`, and the other
  tags stay absent

#### Scenario: legacy string lists are accepted

- **WHEN** a provider's stored model list is still a list of bare strings
- **THEN** it is read as entries whose ids are those strings, each carrying `text` alone,
  with no error and no offline migration step

#### Scenario: tags do not gate requests

- **WHEN** a request targets a model whose tags omit `vision`
- **THEN** the request is routed and forwarded exactly as it would be without any
  capability tag
