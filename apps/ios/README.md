# Vorn for iPhone and iPad

The Rust core (`packages/core/crates/phone`) talks to vornd and builds each screen as a vornui tree. The native backend reads that tree as semantic nodes, and the SwiftUI host here maps every node kind to a platform view. The host knows no screen; a terminal stays a drawn vornui view on Metal.

## Build

```sh
VORN_HOST=192.168.0.14:56097 sh apps/ios/build.sh
```

The script builds `libvorn_phone.a` for the device and the simulator, writes `Vorn/Icons.xcassets` with `icons.py`, generates the project with XcodeGen and builds `Vorn.app`. Set `PROFILE` to sign for a device.

## Icons

`icons.py` turns vornui's own SVGs (`packages/core/crates/vornui/icons`) into template images and the agent marks into full-colour images named `agent-<key>`, so the app draws the same glyphs as the desktop. An extension's mark, unknown at build time, arrives as SVG in the node and is drawn by the core's renderer (`vp_svg_rgba`).

## Routes

A screen is named by a route; the part after the first `:` is its argument.

| Route | Screen |
| --- | --- |
| `sessions`, `tasks`, `workflows` | The tabs |
| `sessions-scope`, `tasks-scope`, `task-new` | Sheets |
| `launch` | A new session |
| `session:<id>` | A terminal |
| `task:<id>`, `workflow:<id>`, `workflow-run:<id>`, `run:<id>` | Details |
| `run-log:<run>[:<node>]`, `review:<run>:<node>` | A run's log, a gate's review |
| `settings`, `settings:<key>` | Settings |

## Node kinds

| Kind | Shows |
| --- | --- |
| `list`, `section`, `column`, `row`, `divider` | Structure |
| `bar` | The navigation title, and its buttons in the toolbar |
| `empty` | An empty state: title, a line saying why, and its buttons |
| `link` | A row that opens the route in `value`; style `sheet` presents it |
| `sheet` | A titled page over the screen |
| `button` | A button, style `plain`, `primary` or `danger`, asking `confirm` first; `value` names its icon |
| `swipe` | Inside a `link`: the buttons shown when the row is swiped from its `style` edge, `leading` or `trailing` |
| `more` | A menu of the buttons inside |
| `choice`, `option` | A picker; segmented without a label, a menu with one |
| `toggle` | A switch |
| `field` | A text field, `multiline` or `secret` |
| `copy` | A button that copies `value` |
| `text` | Text in a role: `title`, `body`, `secondary`, `mono`, `error` |
| `icon` | A glyph: `value` is the vornui name (`agent:<key>` for a mark), `style` its tone (`good`, `warn`, `busy`, `bad`, `muted`, `lit`), `label` inline SVG |
| `status`, `badge` | Small marks |
| `drawn` | A drawn vornui view (a terminal) |

Every control acts by id through `vp_native_action`, and the core says where to go next through `vp_take_go`. `vp_call` sends one request to vornd outside any screen (revoking a token, say).
