## ADDED Requirements

### Requirement: Fetch models from the provider's official base URL

The WebUI provider surface SHALL offer a fetch action that retrieves the model ids the
provider's official base URL currently serves. The action SHALL be available for
preset-derived and custom providers alike, and SHALL be hidden for a provider with no
usable base URL. Running it SHALL persist nothing: the returned ids are shown as a
result list, and an id joins the provider's model list only when the operator picks it,
which is an ordinary edit. Fetched models SHALL start with no capability tags beyond the
implicit text capability, and their parameters SHALL be shown as locally resolved or
unknown rather than inferred from the id. Failures SHALL be reported with the sanitized
reason and SHALL NOT be presented as an empty successful list.

#### Scenario: fetch lists the official model ids

- **WHEN** the operator runs fetch on a preset-derived provider whose base URL serves a model list
- **THEN** the result list shows the returned ids, and the provider's stored data is unchanged until the operator picks one

#### Scenario: picking a fetched model edits the list

- **WHEN** the operator picks a fetched id
- **THEN** that id is added to the provider's model list with the implicit text capability and no invented parameters

#### Scenario: no base URL means no fetch entry

- **WHEN** a provider has no usable base URL
- **THEN** the fetch action is not rendered for it

#### Scenario: failure is reported honestly

- **WHEN** the upstream fetch fails
- **THEN** the surface shows the sanitized reason, and does not display an empty list as if the provider offered no models
