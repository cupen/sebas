## Purpose
Defines the observable visual and interaction contract of the local operator
console: the design token system, the session status vocabulary, live updating
of the session board from the event stream, responsive behavior down to phone
width, the accessibility floor, and the interface copy conventions.

## ADDED Requirements

### Requirement: Single design token source

All color, typography, spacing, and radius values used by the console SHALL
derive from a single declared token set. No page may introduce an ad hoc color
or font size outside that set, and no template may reference a style hook that
has no rule backing it.

#### Scenario: every referenced style hook resolves

- **WHEN** the set of CSS classes referenced across all rendered templates is
  compared against the classes defined in the stylesheet
- **THEN** every referenced class has at least one rule, or the reference is
  removed from the template

#### Scenario: no orphan literals

- **WHEN** a reviewer greps the stylesheet for color literals outside the token
  declaration block
- **THEN** none are found

### Requirement: Session status vocabulary

Each session SHALL render a status drawn from a fixed vocabulary — Starting,
Queued, Working, Done, Failed, Dormant — derived from the session's mapping
state and phase. Each value SHALL carry a distinct human-readable label, a
distinct token color, and a distinct non-color channel (glyph or shape). Color
SHALL never be the only channel distinguishing two statuses. Status labels
SHALL be the console's own vocabulary and SHALL NOT expose upstream transport
identifiers.

#### Scenario: status legible without color

- **WHEN** the session board is rendered in grayscale
- **THEN** each row's status remains identifiable from its label and glyph

#### Scenario: no upstream identifiers surface

- **WHEN** a session's phase is the Feishu reaction token `Get`, `OnIt`, or
  `CrossMark`
- **THEN** the console renders `Queued`, `Working`, or `Failed` respectively,
  and the raw token appears nowhere in the page

#### Scenario: queued distinguishable from working

- **WHEN** a session has acknowledged a prompt but the agent has not begun
  streaming
- **THEN** its status reads as Queued with its own label and glyph, not as
  Working

### Requirement: Live session board

The session board SHALL subscribe to the `/events` stream and update rows in
place when a session's status, phase, or last-active time changes, without a
full page reload. When the stream is unavailable, the board SHALL render current
state as static content and SHALL NOT display a broken or empty board.

#### Scenario: status change lands without reload

- **WHEN** a session transitions from working to done while the board is open
- **THEN** that row's status updates in place and the page is not reloaded

#### Scenario: stream unavailable

- **WHEN** the event stream cannot be established
- **THEN** the board still renders every known session from the initial page
  response

### Requirement: Responsive down to phone width

Every console page SHALL be usable at 375 px viewport width: no horizontal
page scrolling, no clipped controls, and every action reachable. The
navigation SHALL collapse rather than consume the viewport, and tabular
session data SHALL reflow so each session's identity, status, and actions stay
readable.

#### Scenario: phone width has no horizontal scroll

- **WHEN** any console page is rendered at 375 px width
- **THEN** the document does not scroll horizontally and no control is clipped

#### Scenario: session actions reachable on phone

- **WHEN** the session board is viewed at 375 px width
- **THEN** each session's view, focus, and end-session controls are reachable
  and hit-target sized

### Requirement: Accessibility floor

The console SHALL provide a visible keyboard focus indicator on every
interactive element, an accessible name for every control whose visible label
is an icon or glyph alone, and SHALL suppress non-essential animation when the
user requests reduced motion. Body text SHALL meet a 4.5:1 contrast ratio
against its background and large text 3:1, in every supported color scheme.

#### Scenario: keyboard focus is visible

- **WHEN** a user tabs through the session board
- **THEN** each focused control shows a visible focus indicator distinguishable
  from its hover state

#### Scenario: reduced motion honored

- **WHEN** the user agent reports `prefers-reduced-motion: reduce`
- **THEN** row transitions render in a static state rather than animating

#### Scenario: icon-only control has a name

- **WHEN** a control renders only a glyph
- **THEN** it exposes an accessible name describing the action it performs

### Requirement: Color scheme follows the operating system

The console SHALL render in both a light and a dark scheme selected by
`prefers-color-scheme`, with both schemes satisfying the contrast floor and
using the same status vocabulary.

#### Scenario: dark scheme applied

- **WHEN** the user agent reports `prefers-color-scheme: dark`
- **THEN** the console renders in its dark scheme with status colors still
  meeting the contrast floor

### Requirement: Offline asset availability

Every stylesheet, script, and font the console needs to render SHALL be served
from the console's own static path. The console SHALL render fully — including
markdown bodies, syntax-highlighted code blocks, and its own typography — on a
host with no outbound internet access.

#### Scenario: renders with no internet

- **WHEN** the console is loaded on a host with outbound network access blocked
- **THEN** all pages render with their intended typography, and markdown and
  code blocks in session bodies display formatted rather than as raw text

### Requirement: No non-functional controls

The console SHALL NOT present a control or navigation item that cannot perform
the action it appears to offer. A control that is unavailable in the current
state SHALL render in an explicit unavailable state that names why.

#### Scenario: no navigation item without a route

- **WHEN** a user activates any navigation item
- **THEN** the target route responds with a rendered page, never a 404

#### Scenario: unavailable action explains itself

- **WHEN** an admin mutation is unavailable because the control plane is not
  connected
- **THEN** the control renders unavailable with text naming the missing
  configuration rather than failing on click

### Requirement: Consistent page shell

Every rendered page, including the admin cluster, SHALL present the same
navigation, brand, and status chrome, so that navigating between the session
pages and the admin pages does not change the surrounding layout.

#### Scenario: admin pages share the shell

- **WHEN** a user navigates from `/sessions` to `/admin/status`
- **THEN** the navigation and brand chrome are unchanged in position and styling

### Requirement: Interface copy conventions

Console copy SHALL name durations in human-readable units rather than raw
counts, use action labels that match the outcome they produce, state in each
empty view what action fills it, and state in each error both what happened and
what to change. Where a control's effect is narrower than its name suggests,
the adjacent copy SHALL state the limit.

#### Scenario: durations are readable

- **WHEN** the console displays daemon uptime or a session's last-active time
- **THEN** it renders as a human-readable duration, not a raw second count

#### Scenario: focus scope is stated

- **WHEN** the console shows the focused session
- **THEN** adjacent copy states that focus changes only what the console
  displays and does not change message routing

#### Scenario: empty view names the next action

- **WHEN** no sessions exist
- **THEN** the empty view names how to start one
