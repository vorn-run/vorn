# native-look spike

A SwiftUI reproduction of the sessions empty state, to see how a native shell
would look. Colours, radii and spacing come from `src/renderer/theme.css`, the
Tailwind v4 defaults, and the empty-state components.

```sh
swift build
.build/debug/NativeLook                    # open the window
.build/debug/NativeLook --render out.png   # offscreen 2x render, no window
```
