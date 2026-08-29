# Vendored browser assets

Everything the console needs to render is served from `/static/`. No page may
reference an external host — see the `webui-console-ui` requirement "Offline
asset availability".

| file | upstream | version | license |
|---|---|---|---|
| `htmx.min.js` | htmx.org | 1.x (as previously vendored) | BSD-2-Clause |
| `marked.min.js` | cdnjs `marked` | 12.0.1 | MIT |
| `highlight.min.js` | cdnjs `highlight.js` | 11.9.0 | BSD-3-Clause |
| `highlight-github.min.css` | cdnjs `highlight.js` styles | 11.9.0 | BSD-3-Clause |
| `highlight-github-dark.min.css` | cdnjs `highlight.js` styles | 11.9.0 | BSD-3-Clause |

`marked` and `highlight.js` are pinned to the exact versions the templates
loaded from cdnjs before this change, so vendoring them is not a version bump.

Both highlight themes ship because the console follows `prefers-color-scheme`;
the light theme is the default and the dark one is applied inside a
`prefers-color-scheme: dark` block in `style.css`.

## Updating

Re-download at the same path with an explicit version, update the table, and
check that code blocks in a session body still highlight. Do not add a build
step — these are committed files, deliberately.
