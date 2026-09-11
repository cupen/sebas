## Context

See `proposal.md` for motivation. Facts that shape the approach:

- The fetch logic already exists: `sebas-router/src/admin.rs` `probe_provider` /
  `fetch_models` tries the OpenAI chat slot, then the responses slot, then Anthropic
  `/v1/models`, with a 5 s timeout and sanitized errors. It parses **only** `data[].id`
  for both OpenAI and Anthropic envelopes, and `?apply=true` writes the list back.
- Context window and similar parameters never come from upstream; they are resolved from
  the static table in `sebas-router/src/models.rs`, with a `[n]` name suffix able to
  override the table.
- `make-core-own-provider-data` removes the router's provider surface, so the fetch must
  be re-homed in core. That change is a prerequisite; this one assumes it has landed.

## Goals / Non-Goals

**Goals:**

- Fetching is a core capability, available to preset-derived providers.
- The surface never claims parameters the upstream did not provide.

**Non-Goals:**

- Writing any provider field as part of a fetch.
- Deriving capabilities or context windows from upstream responses.
- Background or scheduled refresh.

## Decisions

### D1. The fetch runs in core

Core resolves the provider's base URL and key and performs the upstream read. The router's
`fetch_models` implementation is extracted so core can reuse it rather than reimplementing
the protocol shapes.

*Alternative rejected*: keep the fetch on the router as a read-only view. It is not a read
of router state — it is an action on provider data, and after the ownership change the
provider surface lives in core; leaving one provider operation behind on the router
recreates the split authority this chain is removing.

### D2. Fetched parameters are resolved locally, never inferred

The upstream gives ids. Parameters come from the static table by id; an unknown id gets the
documented default and an explicit "unknown" label.

*Alternative rejected*: parse whatever extra fields an upstream happens to return (for
example `created` or `owned_by`). They are not context windows, and mapping them to
parameters would manufacture information. The honest failure mode is the point.

### D3. Fetch persists nothing; picking is an edit

The fetch returns a list for display. A model joins the provider's list only through the
ordinary edit path, and starts with only the implicit text capability.

*Alternative rejected*: auto-apply the fetched list. It would silently destroy a curated
model list (and, for presets, overwrite code-table-derived data), and the request
explicitly says preset fetching modifies nothing else.

### D4. One bounded, sanitized upstream call

A single request per invocation, 5 s timeout, no retry loop, typed rejections carrying only
a status or category. This preserves the existing sanitization posture.

*Alternative rejected*: retry with backoff across all three URL candidates. It turns one
operator action into a burst against an upstream that just failed, and the current
single-URL choice is already specified behavior.

### D5. A provider-scoped op on the `providers` domain

The fetch is expressed as an operation on the existing provider domain (alongside the
put/delete/save ops) rather than as a new stored domain, because it acts on a provider and
stores nothing.

*Alternative rejected*: a new `models` domain. The channel's domains are storage partitions;
a stateless action does not deserve one.

## Risks / Trade-offs

- [Upstreams without a models endpoint] some Anthropic-protocol deployments serve no
  `/v1/models`. → Mitigation: the existing best-effort posture; the failure surfaces as a
  sanitized reason, never as an empty list.
- [Slow upstream blocks the request] a 5 s upstream call occupies the caller. →
  Mitigation: the existing timeout bound; no retries; the UI shows a pending state.
- [Large catalogs] a provider may return hundreds of ids. → Mitigation: display-only list;
  the operator picks individual entries, so nothing is bulk-written by accident.
- [Key leakage via errors or logs] fetch failures involve a credential. → Mitigation: reuse
  the existing sanitizer and add a test asserting no key material in results or errors.

## Migration Plan

1. Prerequisite: `make-core-own-provider-data` has landed, so the router probe no longer
   exists.
2. Extract the fetch implementation, expose it as the core provider-domain op, and cover it
   with channel tests.
3. Re-home the Feishu card probe and add the WebUI fetch entry.
4. Rollback: revert the binary; the router probe is already gone in the prerequisite
   change, so rolling back past this change alone restores only the UI entry point.

## Open Questions

- Whether fetched lists should be cached per provider to avoid repeated upstream calls is
  deferred; it does not change the specs or the task breakdown.
