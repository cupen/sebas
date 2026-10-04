## ADDED Requirements

### Requirement: Alias creation guides when no provider target exists

The alias creation form SHALL remain usable when the provider store holds no
editable provider (e.g. only the read-only config-seeded provider exists, or
the store is empty): if no selectable target provider exists, the target
dropdown SHALL render a disabled state with visible guidance telling the
operator to create a provider in the models section first. The form SHALL NOT
present a silently empty dropdown, and SHALL NOT allow submitting an alias
without a target.

#### Scenario: empty dropdown shows guidance

- **WHEN** the operator opens the new-alias form and no selectable provider exists
- **THEN** the target dropdown renders disabled with guidance text pointing to the models section

#### Scenario: creating a provider unblocks the form

- **WHEN** the operator creates a provider and reopens the new-alias form
- **THEN** the target dropdown offers the new provider and submission proceeds normally
