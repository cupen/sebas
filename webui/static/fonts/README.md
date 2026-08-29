# Self-hosted fonts

Latin-subset woff2, served from `/static/fonts/`. No Google Fonts link — see the
`webui-console-ui` requirement "Offline asset availability".

| file | family | axis coverage | license |
|---|---|---|---|
| `archivo-latin.woff2` | Archivo | variable weight | OFL 1.1 |
| `archivo-narrow-latin.woff2` | Archivo Narrow | variable weight | OFL 1.1 |
| `martian-mono-latin.woff2` | Martian Mono | variable weight | OFL 1.1 |

Three files, not four: Google serves these families as variable fonts, so one
Archivo file covers both the 400 body weight and the 600 used for emphasis.
Total 57 KB, against the 150 KB budget in the change's design notes.

Subsetting is upstream's — these are the `latin` unicode-range files the Google
Fonts CSS API serves, fetched directly rather than re-subset locally, so no
font tooling is needed to reproduce them.

`@font-face` declarations live in `static/style.css` with `font-display: swap`.
