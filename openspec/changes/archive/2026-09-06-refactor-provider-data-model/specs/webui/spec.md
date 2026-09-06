## ADDED Requirements

### Requirement: Provider management page

The WebUI settings SHALL provide a provider management page backed by the
router admin API (via the WebUI's `/router/api/providers*` proxy). It
SHALL support: listing providers with name, preset-or-custom mark, base
URL slots, and key-configured state; creating a provider either from a
preset or as custom; editing an existing provider; deleting a provider;
and probing models. Preset-derived providers SHALL present base URLs and
models as read-only code-owned values (labeled as following the code
table) with only the API key, default model, and protocol editable;
custom providers SHALL present all three base URL slots and `api_key_env`
as editable. Secret inputs SHALL never be pre-filled, and an empty secret
submit SHALL preserve the stored key. Mutations SHALL go through the
existing POST-only, origin-checked proxy and surface its errors (409
duplicate, 400 validation, 503 unavailable) in the page.

#### Scenario: preset-derived provider shows read-only URLs

- **WHEN** the user opens provider `glm` (preset-derived) for editing
- **THEN** the base URL fields render the preset's code-owned values as
  read-only with a follow-the-code indication, and only the API key,
  default model, and protocol inputs are editable

#### Scenario: create from preset

- **WHEN** the user creates provider `my-glm` from preset `glm` entering
  only an API key
- **THEN** the create request carries the preset selection and key but no
  base URL or models fields, and the provider appears in the list

#### Scenario: custom provider full editing

- **WHEN** the user creates a custom provider filling
  `base_url_openai_chat` only
- **THEN** the create succeeds and the edit form exposes all three slots
  for later adjustment

#### Scenario: probe from the page

- **WHEN** the user clicks probe on a provider with an OpenAI-family slot
- **THEN** the page shows the returned model list without leaving the page

#### Scenario: delete reflects immediately

- **WHEN** the user deletes provider `alpha` from the page
- **THEN** the provider disappears from the list without a page reload
