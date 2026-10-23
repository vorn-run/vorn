# Themes

A theme is an extension whose manifest has a `[theme]` section. The section is data: the colours of Vorn's interface and the terminal's palette. A theme needs no program, and it installs from git like any extension.

Settings › Appearance picks the colour scheme (System, Light or Dark), a light theme and a dark theme from preview tiles, the interface style, contrast correction and materials.

## The manifest

```toml
id = "acme.dusk"
name = "Acme Dusk"
version = "1.0.0"
api = "vorn:extension@1.0"
description = "A warm dark theme."

[theme]
appearance = "dark"          # or "light": which slot it is offered for

[theme.ui]
surface-base = "#1b1d23"
ink = "#e6e1d6"
accent = "#d08770"

[theme.terminal]
foreground = "#e6e1d6"
background = "#1b1d23"
cursor = "#e6e1d6"           # optional, the foreground by default
selection = "#3a3f4b"        # optional, derived by default
ansi = [
  "#1b1d23", "#d0625f", "#9cbc7c", "#e0b86a", "#7f9fc6", "#b58fb8", "#78b7b8", "#d8d3c8",
  "#5c6170", "#e57a77", "#b1d191", "#f0ca80", "#97b6dc", "#cba5cf", "#8ecdce", "#f4efe4",
]
```

`[theme.terminal]` needs the foreground, the background and all 16 ANSI colours. `[theme.ui]` may set any of the interface tokens below; a token it leaves out is derived from the ones it sets and from the terminal palette, so a short `[theme.ui]` is a whole theme. A mistake is reported with its line and field, as `vorn-extension.toml:10: theme.ui.accnet: ...`.

Colours are `#rrggbb`, or `#rrggbbaa` for the tokens drawn over others (lines, selection, shadow).

## Interface tokens

- Surfaces: `surface-base`, `surface-panel`, `surface-sunken`, `surface-overlay`
- Text: `ink`, `ink-bright`, `ink-secondary`, `ink-faint`, `ink-ghost`
- Lines: `line-faint`, `line-soft`, `line-base`, `line-strong`, `line-hover`
- Meaning: `accent`, `accent-strong`, `danger`, `status-slate`, `status-blue`, `status-sage`, `diff-add`, `diff-remove`
- Controls: `selection`, `focus-ring`, `scroll-thumb`, `scroll-thumb-hover`, `slider-track`, `shadow`
- The scales: `gray-100` to `gray-900`, and `red`, `orange`, `amber`, `yellow`, `green`, `emerald`, `cyan`, `sky`, `blue`, `violet`, `purple` and `pink` steps
- Code: `syntax-text`, `syntax-comment`, `syntax-string`, `syntax-number`, `syntax-keyword`, `syntax-storage`, `syntax-constant`, `syntax-function`, `syntax-variable`, `syntax-property`, `syntax-type`, `syntax-punct`, `syntax-list`

Every colour Vorn draws comes from these tokens, on the desktop, in the browser and in the terminal UI, so a theme recolours all of them, live.

## Appearance settings

- **Interface style**: Neutral keeps Vorn's own surfaces and lines with the theme's accent and terminal; Themed draws every token from the theme.
- **Contrast correction**: text that reads below 4.5:1 on its background is lifted until it does, keeping its hue; status colours keep their meaning.
- **Materials**: Solid draws opaque surfaces. Native shows the system's translucent sidebar under a light wash of the theme's panel colour (macOS, Windows 11; solid elsewhere; on Windows it follows at the next launch) and blurs what is behind menus, popovers and the palette, in the theme's colours. Themed does the same with a stronger tint of the theme's surfaces. Reduce transparency in the system's settings keeps everything solid.

## Bundled themes

Vorn Dark (the default) and Vorn Light, Solarized Dark and Light, Nord, Catppuccin Mocha and Latte, and Dracula. Their files are in `packages/core/crates/theme/themes/`, in the same format; their licences are in `THIRD_PARTY_NOTICES.md`.
