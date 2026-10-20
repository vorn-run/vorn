//! A Files pane's tree: what has been listed, which folders are open, the
//! filter, and the rows it draws.

use std::collections::{HashMap, HashSet};

use vorn_extension_sdk::tree::{Line, LineKind};
use vorn_extension_sdk::Value;

/// One entry of a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}

impl Entry {
    pub fn read(v: &Value) -> Option<Entry> {
        Some(Entry {
            name: v.get("name")?.as_str()?.to_owned(),
            path: v.get("path")?.as_str()?.to_owned(),
            dir: v.get("isDirectory").and_then(Value::as_bool) == Some(true),
        })
    }
}

/// The entries of a `files.list` answer.
pub fn entries(v: &Value) -> Vec<Entry> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Entry::read)
        .collect()
}

/// A session's tree.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    /// The folder listed at the top.
    pub cwd: String,
    pub root: Vec<Entry>,
    pub dirs: HashMap<String, Vec<Entry>>,
    pub expanded: HashSet<String>,
    pub filter: String,
}

impl Tree {
    pub fn is_dir(&self, path: &str) -> bool {
        let find = |list: &[Entry]| list.iter().any(|e| e.dir && e.path == path);
        find(&self.root) || self.dirs.values().any(|l| find(l))
    }

    /// Entries that match or hold a match, and the folders opened to show
    /// them; only what has been listed counts.
    fn filter_sets(&self) -> (HashSet<&str>, HashSet<&str>) {
        let (mut matched, mut expand) = (HashSet::new(), HashSet::new());
        if self.filter.is_empty() {
            return (matched, expand);
        }
        let lc = self.filter.to_lowercase();
        fn visit<'a>(
            t: &'a Tree,
            e: &'a Entry,
            lc: &str,
            matched: &mut HashSet<&'a str>,
            expand: &mut HashSet<&'a str>,
        ) -> bool {
            let own = e.name.to_lowercase().contains(lc);
            if !e.dir {
                if own {
                    matched.insert(&e.path);
                }
                return own;
            }
            let mut below = false;
            for c in t.dirs.get(&e.path).into_iter().flatten() {
                below |= visit(t, c, lc, matched, expand);
            }
            if below {
                expand.insert(&e.path);
            }
            if own || below {
                matched.insert(&e.path);
            }
            own || below
        }
        for e in &self.root {
            visit(self, e, &lc, &mut matched, &mut expand);
        }
        (matched, expand)
    }

    /// The rows drawn, depth first; and whether a filter matched nothing listed.
    pub fn rows(&self) -> (Vec<Line>, bool) {
        let (matched, expand) = self.filter_sets();
        let filtering = !self.filter.is_empty();
        let mut out = Vec::new();
        fn walk(
            t: &Tree,
            entries: &[Entry],
            depth: u16,
            filter: Option<(&HashSet<&str>, &HashSet<&str>)>,
            out: &mut Vec<Line>,
        ) {
            for e in entries {
                if filter.is_some_and(|(m, _)| !m.contains(e.path.as_str())) {
                    continue;
                }
                if !e.dir {
                    out.push(Line {
                        id: e.path.clone(),
                        label: e.name.clone(),
                        depth,
                        kind: LineKind::Leaf,
                    });
                    continue;
                }
                let open = t.expanded.contains(&e.path)
                    || filter.is_some_and(|(_, x)| x.contains(e.path.as_str()));
                out.push(Line {
                    id: e.path.clone(),
                    label: e.name.clone(),
                    depth,
                    kind: LineKind::Folder { open, busy: false },
                });
                if !open {
                    continue;
                }
                if let Some(children) = t.dirs.get(&e.path) {
                    if children.is_empty() {
                        out.push(Line {
                            id: String::new(),
                            label: "empty".to_owned(),
                            depth: depth + 1,
                            kind: LineKind::Note,
                        });
                    }
                    walk(t, children, depth + 1, filter, out);
                }
            }
        }
        let filter = filtering.then_some((&matched, &expand));
        walk(self, &self.root, 0, filter, &mut out);
        (out, filtering && matched.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, path: &str, dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: path.into(),
            dir,
        }
    }

    #[test]
    fn the_filter_opens_folders_down_to_what_matches() {
        let mut t = Tree {
            root: vec![e("src", "/src", true), e("README.md", "/README.md", false)],
            ..Tree::default()
        };
        t.dirs.insert(
            "/src".into(),
            vec![
                e("app.ts", "/src/app.ts", false),
                e("lib", "/src/lib", true),
            ],
        );
        t.dirs.insert("/src/lib".into(), Vec::new());
        assert_eq!(t.rows().0.len(), 2);
        t.expanded.insert("/src".into());
        t.expanded.insert("/src/lib".into());
        let (rows, _) = t.rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[3].kind, LineKind::Note);
        assert_eq!(rows[3].depth, 2);
        t.filter = "APP".into();
        t.expanded.clear();
        let (rows, none) = t.rows();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(!none);
        assert_eq!(
            rows[0].kind,
            LineKind::Folder {
                open: true,
                busy: false
            }
        );
        t.filter = "zzz".into();
        let (rows, none) = t.rows();
        assert!(rows.is_empty() && none);
        assert!(t.is_dir("/src/lib") && !t.is_dir("/README.md"));
    }
}
