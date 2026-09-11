## Context

See `proposal.md` for motivation. This change sits at the end of a chain and assumes two
prerequisites have landed: `make-core-own-provider-data` (core owns the provider store,
the router has no write path) and `add-fetch-models` (the fetch capability and its
contract). Facts that shape the approach:

- `ProviderConfig.models` is `Vec<String>` (`sebas-router/src/config.rs`) and its order is
  load-bearing: `default_model()` is the first element, and `map_to_env` maps model entry
  ids onto Claude Code's OPUS/SONNET/HAIKU variables (`models.rs`).
- The static preset table carries `models: &'static [&'static str]`, so it needs tags too.
- The only per-model attributes today are `context_window` / `max_output_tokens` from the
  static registry; no capability notion exists.
- The provider editor is a `<wa-dialog>`; Web Awesome's `<wa-select>` dispatches a bubbling,
  composed `wa-hide` when its own listbox closes, and the dialog's handler does not check
  the event source. The same unguarded pattern recurs on five sibling dialogs.

## Goals / Non-Goals

**Goals:**

- A provider holds any number of model entries, each with capability tags.
- The editor is usable: no interaction with an inner control may close it.

**Non-Goals:**

- Implementing the fetch mechanism (owned by `add-fetch-models`).
- Moving provider data ownership (owned by `make-core-own-provider-data`).
- Making capability tags affect routing or request acceptance.

## Decisions

### D1. Model entries are objects, and legacy strings still read

The list becomes a list of entries carrying `id` and its tags. A compatibility
deserializer accepts a bare string as an entry with no explicit tags, and every write
emits entries.

*Alternative rejected*: a parallel `model_caps` map keyed by model id. It keeps the
element type but ties tags to an id rather than to the entry, so it cannot express the same
id twice and splits one editable thing across two structures — and the request explicitly
asked for the model itself to stop being a string.

*Consequence*: the compatibility read is what avoids an offline migration. Ordering stays
meaningful (first entry is the default model), so entry order is preserved on write.

### D2. Capability vocabulary is `text` + `vision` / `audio` / `video`

`text` is implied and never stored; the other three are stored explicitly and are
per-entry.

*Alternatives rejected*: `text` + `vision` only (the request named more than one
modality); adding tool-calling or reasoning (those describe behavior rather than input
modality, and mixing the two axes invites the reader to expect enforcement).

### D3. `api_key_env` leaves the form, not the model

The UI stops offering it; the field stays in the provider model because presets source
their default key from it and the spawn path resolves plaintext over env. Removing the
concept would force every preset to carry a plaintext key.

*Consequence*: an env var still works as a silent fallback when no plaintext key is
stored; the UI reports configured/unconfigured honestly.

### D4. Custom form maps one URL to the slot its protocol names

Custom create collects instance name, one `base_url`, and a wire protocol (Anthropic /
OpenAI-compatible, defaulting to OpenAI-compatible); the other two slots live under
Advanced.

*Alternative rejected*: writing the one URL into every slot. The Anthropic and OpenAI
paths differ, so that would mis-route rather than simplify.

### D5. Preset instance name defaults to the preset name

A preset create stores the provider under the preset name; the name field lives in
Advanced. A second instance of the same preset is a custom provider.

*Alternative rejected*: always prompting for a name, which buys duplicate vendor accounts
at the cost of the minimal path the request asks for.

### D6. Guard every dialog hide by event source

Every `@wa-hide` binding in the settings modal becomes source-checked
(`e.target === e.currentTarget`), covering the provider editor, set-default, delete,
service-confirm, restart-all, and reset dialogs.

*Alternative rejected*: `stopPropagation` on the select's hide event — it fixes the
observed path only, leaves the siblings broken, and depends on library internals.

### D7. This change consumes the fetch contract rather than defining it

The form renders a fetch entry and result list; the mechanics (endpoint, persistence
posture, honest limits) belong to `add-fetch-models`.

*Alternative rejected*: inlining fetch semantics here. Two capabilities would then specify
one behavior, and they would drift.

## Risks / Trade-offs

- [Shape change ripples through ordering and env mapping] `default_model`, `map_to_env`,
  the preset table, and persistence all read the list. → Mitigation: keep entry order
  authoritative, add the compatibility read, and cover the ripple with unit tests before
  touching the UI.
- [Rollback hazard] an older binary deserializing entry objects into `Vec<String>` may
  reject stored data. → Mitigation: state it in the release note; the forward migration is
  safe, the backward one needs the pre-change binary to tolerate or be cleared. Accept the
  risk rather than keeping a compatibility write path.
- [Existing e2e encodes the old contract] `tests/testsuite-webui/tests/models.spec.ts`
  asserts read-only providers and zero probe traffic. → Mitigation: update it in the same
  change; a passing unchanged spec is a false green.
- [Metadata mistaken for enforcement] operators may expect a `text`-only entry to reject
  image input. → Mitigation: the spec states tags never alter routing; the UI labels them
  as annotations.
- [Chain coupling] the transport wording in `Provider management page` and the Services
  block assume the ownership change landed. → Mitigation: re-run validation against the
  archived predecessors before applying this change.

## Migration Plan

1. Land the entry shape with the compatibility read first, so stored data needs no offline
   migration.
2. Ship the dialog guard independently of the form redesign.
3. Rework the form, then remove the gateway card and add the Services coverage.
4. Rollback: reverting the binary restores string handling; entries already written need the
   older binary to tolerate them (see the rollback hazard above).

## Open Questions

- Whether per-model parameters (context window and friends) should become editable rather
  than registry-resolved is deferred; it does not change the specs or the task breakdown.
- Whether the vocabulary should later grow a non-modality axis (tool-calling) is deferred.
