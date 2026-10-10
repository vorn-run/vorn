"""Applies the spike's test-support hooks to the pinned GPUI checkout: present
without the profiler, the input handler for IME, an active accessibility
tree, and the test window's scale factor from VORN_SPIKE_SCALE.

Idempotent; run by fetch_zed.sh after checkout.
"""

import sys

ZED = sys.argv[1]

EDITS = [
    (
        "crates/gpui/src/window.rs",
        """    /// Presents the most recently drawn frame if it hasn't been presented yet.
    #[cfg(all(test, feature = "profiler"))]
    pub fn present_if_needed(&mut self) {
        if self.needs_present.get() {
            self.present();
        }
    }
""",
        """    /// Presents the most recently drawn frame if it hasn't been presented yet.
    #[cfg(any(test, feature = "test-support"))]
    pub fn present_if_needed(&mut self) {
        if self.needs_present.get() {
            self.present();
        }
    }

    /// Takes the platform's input handler, so a test can drive it as an OS IME would.
    #[cfg(any(test, feature = "test-support"))]
    pub fn take_platform_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.platform_window.take_input_handler()
    }

    /// Gives back a handler taken with [`Self::take_platform_input_handler`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_platform_input_handler(&mut self, handler: PlatformInputHandler) {
        self.platform_window.set_input_handler(handler);
    }
""",
    ),
    (
        "crates/gpui/src/platform/test/window.rs",
        """            scale_factor: 2.0,
""",
        """            scale_factor: std::env::var("VORN_SPIKE_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(2.0),
""",
    ),
    (
        "crates/gpui/src/platform/test/window.rs",
        """    fn draw(&self, scene: &Scene) {""",
        """    fn a11y_init(&self, callbacks: crate::A11yCallbacks) {
        // Act as if a screen reader is connected, so frames build the tree.
        (callbacks.activation)();
    }

    fn draw(&self, scene: &Scene) {""",
    ),
]

for rel, old, new in EDITS:
    path = f"{ZED}/{rel}"
    with open(path, encoding="utf-8") as f:
        src = f.read()
    if new in src:
        continue
    if src.count(old) != 1:
        sys.exit(f"patch_zed: anchor not found once in {rel}")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(src.replace(old, new))
