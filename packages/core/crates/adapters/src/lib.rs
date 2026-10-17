//! The agent adapters Vorn ships. Each is an extension package of its own
//! under `agents/<name>/`: a `vorn-extension.toml` whose `[agent]` section
//! says how to run one agent CLI and its mark.
//! Vorn reads them through the same format an installed package uses; none
//! of them carries code.

/// One shipped adapter.
#[derive(Debug)]
pub struct Shipped {
    /// Its folder's name.
    pub name: &'static str,
    /// Its `vorn-extension.toml`.
    pub manifest: &'static str,
    /// Whether it is on before the person switches it.
    pub on_by_default: bool,
    /// Every other file, by its path in the package, as `icon.svg`.
    pub files: &'static [(&'static str, &'static [u8])],
}

impl Shipped {
    /// The file at `path` in the package, if it ships one.
    pub fn file(&self, path: &str) -> Option<&'static [u8]> {
        self.files.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
    }
}

/// Every adapter Vorn ships, in the order the launcher lists them.
pub fn shipped() -> &'static [Shipped] {
    include!(concat!(env!("OUT_DIR"), "/shipped.rs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_each_package_with_its_mark() {
        assert!(!shipped().is_empty());
        for s in shipped() {
            assert!(s.manifest.contains("[agent]"), "{}", s.name);
            assert!(s.file("icon.svg").is_some(), "{} has no icon", s.name);
            assert!(s.file("vorn-extension.toml").is_none());
        }
    }
}
