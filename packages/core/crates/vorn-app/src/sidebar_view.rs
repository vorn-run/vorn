//! The sessions sidebar's view options: what it groups sessions by, the
//! order they take, which fields each row shows, and whether empty groups
//! show. They outlive a launch in `desktop/sidebar.json`.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// What the sidebar groups sessions under.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum GroupBy {
    /// One list.
    None,
    Project,
    /// Each project, then its main checkout and worktrees.
    #[default]
    Worktree,
    /// The person's own session groups.
    Groups,
    /// A grouping an extension contributes, by its point's key.
    Point(String),
}

impl GroupBy {
    /// The app's own groupings, in the menu's order.
    pub const BUILT_IN: [GroupBy; 4] = [
        GroupBy::None,
        GroupBy::Project,
        GroupBy::Worktree,
        GroupBy::Groups,
    ];

    pub fn label(&self) -> &str {
        match self {
            GroupBy::None => "None",
            GroupBy::Project => "Project",
            GroupBy::Worktree => "Worktree",
            GroupBy::Groups => "My groups",
            GroupBy::Point(_) => "Extension",
        }
    }

    /// What its menu entry says when hovered.
    pub fn tip(&self) -> &str {
        match self {
            GroupBy::None => "Lists every session in one list",
            GroupBy::Project => "Groups sessions under their project",
            GroupBy::Worktree => "Groups sessions under their project and worktree",
            GroupBy::Groups => "Groups sessions under the groups you make",
            GroupBy::Point(_) => "Groups sessions as the extension says",
        }
    }

    fn key(&self) -> String {
        match self {
            GroupBy::None => "none".into(),
            GroupBy::Project => "project".into(),
            GroupBy::Worktree => "worktree".into(),
            GroupBy::Groups => "groups".into(),
            GroupBy::Point(k) => format!("point:{k}"),
        }
    }

    fn parse(key: &str) -> Option<GroupBy> {
        Some(match key {
            "none" => GroupBy::None,
            "project" => GroupBy::Project,
            "worktree" => GroupBy::Worktree,
            "groups" => GroupBy::Groups,
            _ => GroupBy::Point(key.strip_prefix("point:")?.to_owned()),
        })
    }

    /// The grouping an old view mode becomes: both worktree modes group by
    /// worktree, the rest by what they listed.
    fn from_mode(mode: &str) -> GroupBy {
        match mode {
            "sessions" => GroupBy::Project,
            "groups-sessions" => GroupBy::Groups,
            "sessions-flat" => GroupBy::None,
            _ => GroupBy::Worktree,
        }
    }
}

impl Serialize for GroupBy {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for GroupBy {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<GroupBy, D::Error> {
        let key = String::deserialize(d)?;
        GroupBy::parse(&key).ok_or_else(|| serde::de::Error::custom("an unknown grouping"))
    }
}

/// The order sessions and their groups take.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Order {
    /// vornd's order, which a drag changes.
    #[default]
    Manual,
    /// Newest first.
    Created,
    /// The latest to start, wait or finish first; output alone never moves a row.
    LastChange,
}

impl Order {
    pub const ALL: [Order; 3] = [Order::Manual, Order::Created, Order::LastChange];

    pub fn label(self) -> &'static str {
        match self {
            Order::Manual => "Manual",
            Order::Created => "Created",
            Order::LastChange => "Last change",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Order::Manual => "Keeps the order you drag sessions into",
            Order::Created => "Puts the newest sessions first",
            Order::LastChange => {
                "Puts sessions that last started, waited or finished first; output never moves them"
            }
        }
    }

    /// The key a session, or a group by its sessions, sorts on: smaller first.
    /// `None` (a group with no sessions) goes last in the time orders.
    pub fn rank(self, created: Option<f64>, changed: Option<f64>) -> Rank {
        match self {
            Order::Manual => Rank(0),
            Order::Created => Rank::newest(created),
            Order::LastChange => Rank::newest(changed),
        }
    }
}

/// Where an item sorts under an [`Order`]: smaller first, ties keep their
/// manual order (sorts on it are stable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank(i64);

impl Rank {
    fn newest(at: Option<f64>) -> Rank {
        // Whole milliseconds: two stamps a fraction apart are the same moment.
        Rank(at.map_or(i64::MAX, |ms| -(ms as i64)))
    }
}

/// Which fields each row's second line shows; an empty field drops out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fields {
    pub branch: bool,
    /// What the program reports of itself.
    pub status: bool,
    pub progress: bool,
    /// The agent's mark leads the row instead of the status dot.
    pub agent: bool,
}

impl Default for Fields {
    fn default() -> Fields {
        Fields {
            branch: true,
            status: false,
            progress: false,
            agent: true,
        }
    }
}

/// One field a row can show, for the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Branch,
    Status,
    Progress,
    Agent,
}

impl Field {
    pub const ALL: [Field; 4] = [Field::Branch, Field::Status, Field::Progress, Field::Agent];

    pub fn label(self) -> &'static str {
        match self {
            Field::Branch => "Branch",
            Field::Status => "Status text",
            Field::Progress => "Progress",
            Field::Agent => "Agent icon",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Field::Branch => "Shows the session's branch under its name",
            Field::Status => "Shows what the program says it is doing",
            Field::Progress => "Shows how far the program says it is",
            Field::Agent => "Leads each row with the agent's icon instead of a status dot",
        }
    }

    pub fn on(self, f: &Fields) -> bool {
        match self {
            Field::Branch => f.branch,
            Field::Status => f.status,
            Field::Progress => f.progress,
            Field::Agent => f.agent,
        }
    }

    pub fn toggle(self, f: &mut Fields) {
        let v = match self {
            Field::Branch => &mut f.branch,
            Field::Status => &mut f.status,
            Field::Progress => &mut f.progress,
            Field::Agent => &mut f.agent,
        };
        *v = !*v;
    }
}

impl Fields {
    /// A row's second line: the picked fields that have something, joined
    /// by a middle dot. Empty when none has.
    pub fn line(&self, branch: Option<&str>, status: Option<&str>, progress: Option<u8>) -> String {
        let progress = progress.map(|p| format!("{p}%"));
        let parts = [
            branch.filter(|_| self.branch),
            status.filter(|_| self.status),
            progress.as_deref().filter(|_| self.progress),
        ];
        let mut out = String::new();
        for part in parts.into_iter().flatten().map(str::trim) {
            if part.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push_str(" \u{b7} ");
            }
            out.push_str(part);
        }
        out
    }
}

/// The options together, as they are saved. A value this build does not know
/// reads as its default, so one stale key never resets the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidebarView {
    #[serde(deserialize_with = "lenient")]
    pub group_by: GroupBy,
    #[serde(deserialize_with = "lenient")]
    pub order: Order,
    #[serde(deserialize_with = "lenient")]
    pub fields: Fields,
    /// Hides ended sessions, and groups with no session.
    pub only_active: bool,
}

fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

impl SidebarView {
    /// What lights the Filter & sort button.
    pub fn non_default(&self) -> bool {
        *self != SidebarView::default()
    }

    /// The options saved in `file`, else the defaults. A file from before
    /// Group by (its `viewMode`) is read through once and written back.
    pub fn load(file: &Path) -> SidebarView {
        let Some(saved) = crate::keep::read(file).and_then(|s| serde_json::from_str(&s).ok())
        else {
            return SidebarView::default();
        };
        let (view, migrated) = SidebarView::read(&saved);
        if migrated {
            view.save(file);
        }
        view
    }

    /// The options in `saved`, and whether they came from the old view modes.
    fn read(saved: &Value) -> (SidebarView, bool) {
        let mode = saved.get("viewMode").and_then(Value::as_str);
        match mode.filter(|_| saved.get("groupBy").is_none()) {
            Some(mode) => (
                SidebarView {
                    group_by: GroupBy::from_mode(mode),
                    only_active: saved.get("worktreeFilter").and_then(Value::as_str)
                        == Some("active"),
                    ..SidebarView::default()
                },
                true,
            ),
            None => (
                serde_json::from_value(saved.clone()).unwrap_or_default(),
                false,
            ),
        }
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
    fn the_old_modes_map_once_to_a_grouping() {
        for (mode, want) in [
            ("worktrees", GroupBy::Worktree),
            ("worktrees-sessions", GroupBy::Worktree),
            ("sessions", GroupBy::Project),
            ("groups-sessions", GroupBy::Groups),
            ("sessions-flat", GroupBy::None),
        ] {
            let saved = serde_json::json!({ "viewMode": mode, "worktreeFilter": "active" });
            let (v, migrated) = SidebarView::read(&saved);
            assert!(migrated, "{mode}");
            assert_eq!(v.group_by, want, "{mode}");
            assert!(v.only_active, "{mode}");
        }
        let (v, migrated) =
            SidebarView::read(&serde_json::json!({ "groupBy": "groups", "viewMode": "worktrees" }));
        assert!(!migrated);
        assert_eq!(v.group_by, GroupBy::Groups);
    }

    #[test]
    fn an_unknown_value_falls_back_alone() {
        let v: SidebarView =
            serde_json::from_str(r#"{"groupBy":"tree","order":"lastChange"}"#).unwrap();
        assert_eq!(v.group_by, GroupBy::Worktree);
        assert_eq!(v.order, Order::LastChange);
    }

    #[test]
    fn an_extension_grouping_round_trips() {
        let v = SidebarView {
            group_by: GroupBy::Point("vorn.status/groupings/status".into()),
            ..SidebarView::default()
        };
        let text = serde_json::to_string(&v).unwrap();
        assert!(
            text.contains(r#""groupBy":"point:vorn.status/groupings/status""#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<SidebarView>(&text).unwrap(), v);
    }

    #[test]
    fn the_second_line_keeps_only_picked_fields_with_something_in_them() {
        let f = Fields::default();
        assert!(f.branch && f.agent && !f.status && !f.progress, "{f:?}");
        assert_eq!(f.line(Some("main"), Some("Building"), Some(60)), "main");
        assert_eq!(f.line(None, Some("Building"), None), "");
        let status = Fields {
            status: true,
            ..Fields::default()
        };
        assert_eq!(
            status.line(Some("main"), Some("Building"), Some(60)),
            "main \u{b7} Building"
        );
        assert_eq!(status.line(Some(" "), Some(""), None), "");
        let all = Fields {
            progress: true,
            ..Fields::default()
        };
        assert_eq!(all.line(Some("main"), None, Some(60)), "main \u{b7} 60%");
    }

    #[test]
    fn time_orders_put_the_newest_first_and_the_empty_last() {
        let mut items = [(1, Some(10.0)), (2, None), (3, Some(30.0)), (4, Some(10.4))];
        items.sort_by_key(|(_, at)| Order::LastChange.rank(None, *at));
        let ids: Vec<i32> = items.iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, [3, 1, 4, 2], "equal moments keep their manual order");
        items.sort_by_key(|(_, at)| Order::Manual.rank(None, *at));
        assert_eq!(
            items.map(|(i, _)| i),
            [3, 1, 4, 2],
            "manual keeps what it is given"
        );
    }

    #[test]
    fn saves_and_loads_through_a_file_migrating_once() {
        let dir = std::env::temp_dir().join(format!("vorn-sidebar-{}", std::process::id()));
        let f = file(&dir);
        std::fs::create_dir_all(dir.join("desktop")).unwrap();
        std::fs::write(&f, r#"{"viewMode":"groups-sessions","projectSort":"name"}"#).unwrap();
        let v = SidebarView::load(&f);
        assert_eq!(v.group_by, GroupBy::Groups);
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains(r#""groupBy":"groups""#), "{text}");
        assert!(!text.contains("viewMode"), "{text}");
        assert_eq!(SidebarView::load(&f), v);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(SidebarView::load(&f), SidebarView::default());
    }
}
