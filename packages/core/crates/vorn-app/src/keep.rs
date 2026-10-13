//! The app's own small files (open panes, drafts, the sidebar's options):
//! on disk beside vornd's data, or in the page's storage in a browser.

use std::path::Path;

/// What `file` holds, if it can be read.
pub fn read(file: &Path) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    return std::fs::read_to_string(file).ok();
    #[cfg(target_arch = "wasm32")]
    crate::page::load(&crate::page::key_for(file))
}

/// Writes `body` to `file`, making its directory first.
pub fn write(file: &Path, body: &str) -> std::io::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, body)
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::page::save(&crate::page::key_for(file), body);
        Ok(())
    }
}
