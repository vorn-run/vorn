# Turns

An example extension, written in Rust and run as a Wasm component, that draws an element tree in every place an extension can draw: a sidebar section, card badges, footers and sidebar row marks, a pane, the new-session composer, a settings page, a window and a dialog. It also adds palette commands, a card menu item and a launcher. The same trees draw in the desktop app, the browser, a terminal and the phone.

```sh
rustup target add wasm32-wasip2
vorn ext link --watch examples/extensions/turns
```

`vorn ext link` runs the manifest's `[build]` first, and `--watch` builds and reloads it on every save. `cargo test` runs its logic against a fake Vorn.

## How it works

It listens to `session.created`, `session.status` and `session.exited`, counts a turn each time a running session goes idle or waits, and keeps everything in the rows it publishes (`sessions`, `finished`, `prefs`, `plans`, `account`). Vorn may stop an idle instance and start a fresh one; `init` publishes its defaults only when the rows are empty, so a restart finds everything as it was.

Every contribution says `tree = true`. Vorn asks for each tree with `ui.render` and draws it on each surface, again whenever the extension's rows change; a click, a toggle, a choice or a field's Return comes back as `ui.event`, whose answer can show a toast, open its dialog or another of its windows, close the dialog or window it came from, bring a session forward, or open a sign-in window.

- **Badges, card footers and sidebar row marks** draw once per session: the tree's root holds one child per session, its id the session's id (`per_session` builds it), and each card draws its own.
- **The settings page** shows the inputs a tree has: a `toggle`, a `choice`, a `field` with `secret` that draws its text as bullets, an entry whose primary `Sign in` answers `{signin: {...}}` to open Vorn's sign-in window, and a `more` menu whose `Forget account` asks first.
- **The window** (`Mod+Alt+T`, or the launcher) answers `{focus: <session>}` from a row's `Show`, and `{show: true}` from `Vorn`.
- **The dialog** opens when an action answers `{open: "clear"}`; its buttons answer `{close: true}`.
