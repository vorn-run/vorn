//! The sessions sidebar's view options: how it
//! arranges sessions, how it orders projects and worktrees, and which it
//! shows. They outlive a launch in `desktop/sidebar.json`.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::state::SidebarMode;

/// `ProjectSortMode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectSort {
    #[default]
    Manual,
    Name,
    Recent,
}

impl ProjectSort {
    pub const ALL: [ProjectSort; 3] = [ProjectSort::Manual, ProjectSort::Name, ProjectSort::Recent];

    pub fn label(self) -> &'static str {
        match self {
            ProjectSort::Manual => "Manual",
            ProjectSort::Name => "Name",
            ProjectSort::Recent => "Recent activity",
        }
    }
}

/// `WorktreeSortMode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeSort {
    #[default]
    Name,
    Recent,
}

impl WorktreeSort {
    pub const ALL: [WorktreeSort; 2] = [WorktreeSort::Name, WorktreeSort::Recent];

    pub fn label(self) -> &'static str {
        match self {
            WorktreeSort::Name => "Name",
            WorktreeSort::Recent => "Recent activity",
        }
    }
}

/// `WorktreeFilter`: every project and worktree, or only those running
/// sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeFilter {
    #[default]
    All,
    Active,
}

impl WorktreeFilter {
    pub const ALL: [WorktreeFilter; 2] = [WorktreeFilter::All, WorktreeFilter::Active];

    pub fn label(self) -> &'static str {
        match self {
            WorktreeFilter::All => "All",
            WorktreeFilter::Active => "With active sessions",
        }
    }
}

/// The options together, as they are saved. A value this build does not know
/// reads as its default, so one stale key never resets the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarView {
    #[serde(deserialize_with = "lenient")]
    pub view_mode: SidebarMode,
    #[serde(deserialize_with = "lenient")]
    pub project_sort: ProjectSort,
    #[serde(deserialize_with = "lenient")]
    pub worktree_sort: WorktreeSort,
    #[serde(deserialize_with = "lenient")]
    pub worktree_filter: WorktreeFilter,
}

fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

impl SidebarView {
    /// What lights the Filter & sort button. It measures
    /// the mode against `worktrees`, not the default arrangement, as today.
    pub fn non_default(&self) -> bool {
        self.view_mode != SidebarMode::Worktrees
            || self.project_sort != ProjectSort::Manual
            || self.worktree_sort != WorktreeSort::Name
            || self.worktree_filter != WorktreeFilter::All
    }

    /// Whether the mode lists projects, so the filter and the project order
    /// apply.
    pub fn by_project(&self) -> bool {
        !matches!(self.view_mode, SidebarMode::Groups | SidebarMode::Flat)
    }

    /// Whether the mode lists worktrees, so their order applies.
    pub fn by_worktree(&self) -> bool {
        matches!(
            self.view_mode,
            SidebarMode::Worktrees | SidebarMode::WorktreesSessions
        )
    }

    /// The options saved in `file`, else the defaults.
    pub fn load(file: &Path) -> SidebarView {
        crate::keep::read(file)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Writes the options down; a failure costs only their survival past
    /// this launch.
    pub fn save(&self, file: &Path) {
        if let Ok(s) = serde_json::to_string(self) {
            let _ = crate::keep::write(file, &s);
        }
    }
}

/// Where the sidebar's options live beside `data_dir`'s vornd.
pub fn file(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop").join("sidebar.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_renderers_keys_and_values() {
        let v: SidebarView = serde_json::from_str(
            r#"{"viewMode":"sessions-flat","projectSort":"recent","worktreeSort":"recent","worktreeFilter":"active","workflowFilter":"all"}"#,
        )
        .unwrap();
        assert_eq!(v.view_mode, SidebarMode::Flat);
        assert_eq!(v.project_sort, ProjectSort::Recent);
        assert_eq!(v.worktree_sort, WorktreeSort::Recent);
        assert_eq!(v.worktree_filter, WorktreeFilter::Active);
    }

    #[test]
    fn an_unknown_value_falls_back_alone() {
        let v: SidebarView =
            serde_json::from_str(r#"{"viewMode":"tree","projectSort":"name"}"#).unwrap();
        assert_eq!(v.view_mode, SidebarMode::WorktreesSessions);
        assert_eq!(v.project_sort, ProjectSort::Name);
    }

    #[test]
    fn the_dot_measures_against_the_worktrees_mode() {
        assert!(SidebarView::default().non_default());
        let plain = SidebarView {
            view_mode: SidebarMode::Worktrees,
            ..SidebarView::default()
        };
        assert!(!plain.non_default());
    }

    #[test]
    fn saves_and_loads_through_a_file() {
        let dir = std::env::temp_dir().join(format!("vorn-sidebar-{}", std::process::id()));
        let f = file(&dir);
        let v = SidebarView {
            view_mode: SidebarMode::Groups,
            worktree_filter: WorktreeFilter::Active,
            ..SidebarView::default()
        };
        v.save(&f);
        assert_eq!(SidebarView::load(&f), v);
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains(r#""viewMode":"groups-sessions""#), "{text}");
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(SidebarView::load(&f), SidebarView::default());
    }
}
