# Vorn for iPhone and iPad

The Rust core (`packages/core/crates/phone`) talks to vornd and builds each screen as a vornui tree. The native backend reads that tree as semantic nodes, and the SwiftUI host here maps every node kind to a platform view. The host knows no screen; a terminal stays a drawn vornui view on Metal.

## Build

```sh
VORN_HOST=192.168.0.14:56097 sh apps/ios/build.sh
```

The script builds `libvorn_phone.a` for the device and the simulator, writes `Vorn/Icons.xcassets` with `icons.py`, generates the project with XcodeGen and builds `Vorn.app`. Set `PROFILE` to sign for a device.

## Icons

`icons.py` turns vornui's own SVGs (`packages/core/crates/vornui/icons`) into template images, so the app draws the same glyphs as the desktop. An agent's mark comes from its adapter (`agent:list`) and an extension's from its manifest; neither is known at build time, so each arrives as SVG in the node and is drawn by the core's renderer (`vp_svg_rgba`).

## Routes

A screen is named by a route; the part after the first `:` is its argument.

| Route                                                                      | Screen                                                         |
| -------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `sessions`, `tasks`, `workflows`                                           | The tabs                                                       |
| `sessions-scope`, `tasks-scope`, `sessions-view`, `tasks-view`, `task-new` | Sheets; the view options are kept across launches (`vp_prefs`) |
| `launch`                                                                   | A new session                                                  |
| `session:<id>`                                                             | A terminal                                                     |
| `task:<id>`, `workflow:<id>`, `workflow-run:<id>`, `run:<id>`              | Details                                                        |
| `run-log:<run>[:<node>]`, `review:<run>:<node>`                            | A run's log, a gate's review                                   |
| `settings`, `settings:<key>`                                               | Settings                                                       |

## Node kinds

| Kind                                          | Shows                                                                                                                                                                             |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `list`, `section`, `column`, `row`, `divider` | Structure; a `section` whose first kid is an `icon` wears it in its header                                                                                                        |
| `toolbar`                                     | A row of buttons                                                                                                                                                                  |
| `entry`                                       | A list row: `label` over `value`, its `icon` leading, trailing text and buttons                                                                                                   |
| `notice`                                      | A line that stands out, in the tone `style`                                                                                                                                       |
| `bar`                                         | The navigation title, and its buttons in the toolbar                                                                                                                              |
| `empty`                                       | An empty state: title, a line saying why, and its buttons                                                                                                                         |
| `link`                                        | A row that opens the route in `value`; style `sheet` presents it                                                                                                                  |
| `sheet`                                       | A titled page over the screen                                                                                                                                                     |
| `button`                                      | A button, style `plain`, `primary` or `danger`, asking `confirm` first; `value` names its icon                                                                                    |
| `swipe`                                       | Inside a `link`: the buttons shown when the row is swiped from its `style` edge, `leading` or `trailing`                                                                          |
| `more`                                        | A menu of the buttons inside                                                                                                                                                      |
| `choice`, `option`                            | A picker; segmented without a label, a menu with one. An option's `text` kid is a line under it                                                                                   |
| `toggle`                                      | A switch                                                                                                                                                                          |
| `select`                                      | A box a tap checks, `on` when checked; inside an `entry` the whole row toggles it                                                                                                 |
| `field`                                       | A text field titled `label`, one line unless `multiline`, `secret` hiding what it holds; Return submits it                                                                        |
| `prose`, `key`                                | A Markdown editor: its `field`, a Write/Preview switch, and its `key` kids in the keyboard bar (`vp_markdown_key` applies one)                                                    |
| `copy`                                        | A button that copies `value`                                                                                                                                                      |
| `text`                                        | Text in a role: `title`, `body`, `secondary`, `mono`, `number`, `error`, `markdown`                                                                                               |
| `icon`                                        | A glyph: `value` is the vornui name (`agent:<key>` for an agent's mark, drawn from `label`), `style` its tone (`good`, `warn`, `busy`, `bad`, `muted`, `lit`), `label` inline SVG |
| `status`, `badge`                             | Small marks                                                                                                                                                                       |
| `drawn`                                       | A drawn vornui view (a terminal)                                                                                                                                                  |

Every control acts by id through `vp_native_action`, and the core says where to go next through `vp_take_go`. `vp_call` sends one request to vornd outside any screen (revoking a token, say).
