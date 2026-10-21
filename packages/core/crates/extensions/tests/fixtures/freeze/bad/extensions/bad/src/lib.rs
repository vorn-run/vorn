//! A built-in that reaches past the public API.

use vorn_extension_sdk::bindings::vorn::extension::host;

pub fn reach() {
    // "task.secretly" in a comment is fine.
    let _ = host::call("task.secretly", "{}");
    let _ = host::call("task.list", "{}");
}
