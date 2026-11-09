# SVG sanitize check (DOMPurify + SMIL)

DOMPurify uses the `svg` and `svgFilters` profiles. It keeps the SMIL tags `animate`, `animateTransform`,
`animateMotion`, `set` and `mpath`, plus their timing and value attributes. The script reports every
element or attribute that DOMPurify strips. jsdom's `<body>` wrapper and XML comments are ignored.
The script exits 1 if anything is stripped.

```sh
cd scripts/cw-infuser/sanitize && npm ci
node sanitize-svgs.mjs                # default: ../gas-bench/out/rendered
node sanitize-svgs.mjs some/dir
```
Last run (2026-10-27): 26 renders (13 logos x code 71/98), 0 stripped.
