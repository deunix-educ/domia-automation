# Appearance: customise colours, font and widths

What can be set **without touching the code** (menu, dashboards, widgets) is done in the administration: see [Devices](devices.md) and the [interface](interface.md). This page describes the **style**: colours, font, rounded corners and menu width.

## The `custom.css` file
After its main style sheet, DomiNet loads `deploy/custom/custom.css` if it exists (`CUSTOM_CSS` variable, `./custom:/custom:ro` mount of the production compose). It is **read again on every page load**: a change shows when you reload the browser, **with no restart and no image rebuild**. It is ignored by git: it is a machine setting.

```bash
cd ~/opt/dominet/deploy/custom
cp custom.css.example custom.css
$EDITOR custom.css            # uncomment and edit the variables you want
```
Reload the page (Ctrl+Shift+R if the browser keeps the old version). **Back to the original look**: delete `custom.css`. A missing, unreadable or over-256 KiB file gives an empty sheet: never an error.

## The variables
They go in a `:root { … }` block.

| Variable | Role | Default |
|---|---|---|
| `--bg-base` | page background | `#0d1117` |
| `--bg-surface` | menu and card background | `#1c2128` |
| `--bg-elevated`, `--bg-hover` | raised elements, hover | `#2d333b`, `#373e47` |
| `--accent`, `--accent-dim`, `--accent-glow` | main colour, its dark variant, its glow | `#00d4aa`, `#00a884`, translucent green |
| `--danger`, `--warning`, `--info`, `--success` | state colours (error, warning, information, success) | `#f85149`, `#e3b341`, `#58a6ff`, `#56d364` |
| `--text-primary`, `--text-secondary`, `--text-muted` | text levels | `#f0f6fc`, `#b1bac4`, `#768390` |
| `--border` | lines and borders | `#30363d` |
| `--radius-sm`, `--radius-md`, `--radius-lg` | rounded corners | `6px`, `10px`, `16px` |
| `--sidebar-w` | left menu width | `220px` |
| `--font` | font | `'Segoe UI', system-ui, …` |

Example (orange theme, wider menu):
```css
:root {
    --accent: #ff9f1a;
    --accent-dim: #d98200;
    --sidebar-w: 260px;
}
```
Beyond the variables you can write any CSS rule (for example `.vc-table td { font-size: .85rem; }`); the classes are those of `crates/dominet-web/style/main.css`.

## Good to know
- The theme stays **dark**: there is no light variant. A light background also means changing the text and line colours.
- The sheet is served by DomiNet at `/custom.css`, public (the login page is styled too). It must contain no secret.
- The file must be in `deploy/custom/` (mounted read-only). A change made elsewhere is not seen.
- **Development**: without the `CUSTOM_CSS` variable, `/custom.css` answers an empty sheet.
