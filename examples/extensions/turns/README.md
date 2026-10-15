# Turns

An example extension with a KDL template for every place an extension can draw: a sidebar section, card badges, footers and row decorations, a pane, the new-session composer, a settings page, a window, a dialog, palette commands, a card menu item and a launcher. The same templates draw in the desktop app, the browser and the terminal.

```sh
vorn ext link examples/extensions/turns
```

A linked extension reloads its views as you save them; a template with a mistake shows each error with its file, line and column in the place it would draw. The program (`program.mjs`) needs Node.

Its settings page shows the inputs a template has: a `toggle`, a `choice` over a collection's rows, a `field` with `secret=#true` that draws its text as bullets, and a button with `signin="{row.id}"` that opens Vorn's sign-in window on the target the program's `signin` method answers.
