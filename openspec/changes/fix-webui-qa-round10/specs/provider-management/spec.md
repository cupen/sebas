## ADDED Requirements

### Requirement: Provider and alias mutations are role-gated

Mutations on the provider surface — creating, editing, or deleting providers (including API key material), editing model aliases, and changing the default provider/model selection — SHALL require the `settings.manage` permission (root and admin). Members and viewers are read-only on this surface: the server SHALL reject their mutation attempts with an authorization error, and the WebUI SHALL hide or disable the mutation controls for roles lacking the permission, consistently with how user and service management are already restricted. Read access to the provider list remains available to signed-in roles.

#### Scenario: Member cannot mutate a provider

- **WHEN** a signed-in member attempts to create or edit a provider (via the API directly or the settings UI)
- **THEN** the server rejects the mutation with an authorization error and no store change occurs
- **AND** the settings UI does not offer the mutation controls to that member

#### Scenario: Admin retains full provider management

- **WHEN** a signed-in admin creates, edits, or deletes a provider or alias
- **THEN** the mutation succeeds and is reflected in the provider list

#### Scenario: Viewer sees the provider list read-only

- **WHEN** a signed-in viewer opens the provider settings
- **THEN** the list is visible read-only, with no mutation controls offered
