//! The Extensions page's lists: the tags a listing carries, who made it, the
//! Installed list's sections with each row's one status, and Browse's tiles.

use serde_json::Value;

use super::{
    describe, is_implicit, matches_query, pack_updates, short_repo, Listing, PackState, Plugins,
    Source,
};
use crate::library::connection_connector;
use crate::runs::s;

/// What an extension adds, as its manifest says; a listing may carry several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tag {
    Agent,
    Connection,
    Pane,
    Steps,
    Theme,
    Tools,
}

impl Tag {
    pub const ALL: [Tag; 6] = [
        Tag::Agent,
        Tag::Connection,
        Tag::Pane,
        Tag::Steps,
        Tag::Theme,
        Tag::Tools,
    ];

    pub fn parse(v: &str) -> Option<Tag> {
        Some(match v {
            "agent" => Tag::Agent,
            "connection" => Tag::Connection,
            "pane" => Tag::Pane,
            "steps" => Tag::Steps,
            "theme" => Tag::Theme,
            "tools" => Tag::Tools,
            _ => return None,
        })
    }

    /// The tag on a row.
    pub fn label(self) -> &'static str {
        match self {
            Tag::Agent => "Agent",
            Tag::Connection => "Connection",
            Tag::Pane => "Pane",
            Tag::Steps => "Workflow steps",
            Tag::Theme => "Theme",
            Tag::Tools => "Agent tools",
        }
    }

    /// A section's heading and Browse's filter chip.
    pub fn plural(self) -> &'static str {
        match self {
            Tag::Agent => "Agents",
            Tag::Connection => "Connections",
            Tag::Pane => "Panes",
            Tag::Steps => "Workflow steps",
            Tag::Theme => "Themes",
            Tag::Tools => "Agent tools",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Tag::Agent => "Runs a coding agent in your sessions",
            Tag::Connection => "Signs in to a service and starts workflows from it",
            Tag::Pane => "Adds a pane beside the terminal",
            Tag::Steps => "Adds steps you can use in workflows",
            Tag::Theme => "Changes Vorn's colours and the terminal palette",
            Tag::Tools => "Gives agents tools through Vorn",
        }
    }
}

/// The tags in a `tags` array, in [`Tag`] order; unknown ones are dropped.
pub fn tags(v: Option<&Value>) -> Vec<Tag> {
    let mut out: Vec<Tag> = v
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| Tag::parse(t.as_str()?))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// What a listing's page shows beyond what it does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Showcase {
    pub homepage: Option<String>,
    pub whats_new: Option<String>,
    /// 16:10 pictures: `https` URLs, or files inside an installed pack.
    pub screenshots: Vec<String>,
}

impl Showcase {
    /// A pack's `showcase` object, or a catalog entry's own top-level fields.
    pub fn read(v: Option<&Value>) -> Showcase {
        let Some(v) = v else {
            return Showcase::default();
        };
        let text = |k| s(v, k).filter(|t| !t.is_empty()).map(str::to_owned);
        Showcase {
            homepage: text("homepage"),
            whats_new: text("whatsNew"),
            screenshots: v
                .get("screenshots")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.homepage.is_none() && self.whats_new.is_none() && self.screenshots.is_empty()
    }
}

/// Who made an extension, as its row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Maker {
    Vorn,
    /// Linked or copied from a folder on this machine.
    You,
    Other(String),
}

impl Maker {
    pub fn label(&self) -> String {
        match self {
            Maker::Vorn => "By Vorn".to_owned(),
            Maker::You => "By you".to_owned(),
            Maker::Other(who) => format!("By {who}"),
        }
    }

    pub fn of(l: &Listing) -> Maker {
        let pack = l.pack.as_ref();
        let local = pack.is_some_and(|p| {
            p.get("linked") == Some(&Value::Bool(true))
                || p.pointer("/source/path").is_some_and(Value::is_string)
        });
        if local {
            return Maker::You;
        }
        match l.publisher.as_deref().filter(|p| !p.is_empty()) {
            Some("Vorn") => return Maker::Vorn,
            Some(who) => return Maker::Other(who.to_owned()),
            None => {}
        }
        // Vorn's own catalog and connectors are Vorn's; a pack from elsewhere is its repository's.
        match pack
            .and_then(|p| p.pointer("/source/git"))
            .and_then(Value::as_str)
        {
            Some(git) if l.source == Source::Installed && !git.contains("/vorn-run/") => {
                Maker::Other(short_repo(git))
            }
            _ if l.source == Source::Installed => Maker::You,
            _ => Maker::Vorn,
        }
    }
}

/// The one thing a row says on its right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// It needs the person: what to do.
    Attention(String),
    Update,
    /// Signed in: the account it uses, and how many more.
    Account(String),
    /// Installing, removing or updating now.
    Busy(String),
    /// A quiet word: Default, On, Off, Ready.
    Note(&'static str),
}

impl Status {
    /// The words the row shows.
    pub fn label(&self) -> &str {
        match self {
            Status::Attention(s) | Status::Account(s) | Status::Busy(s) => s,
            Status::Update => "Update",
            Status::Note(s) => s,
        }
    }

    pub fn tip(&self) -> &'static str {
        match self {
            Status::Attention(_) => "Opens it to fix what's missing",
            Status::Update => "Opens it to see what's new and update",
            Status::Account(_) => "Signed in and watching for work",
            Status::Busy(_) => "Working on it now",
            Status::Note("Default") => "Starts in new sessions unless you pick another",
            Status::Note("Off") => "Turned off; open it to turn it back on",
            Status::Note("Ready") => "Works without signing in",
            Status::Note(_) => "Installed and running",
        }
    }
}

/// A row of the Installed list: a built-in extension or a listing.
#[derive(Debug, Clone, Copy)]
pub enum Item<'a> {
    Builtin(&'a Value),
    Listing(&'a Listing),
}

/// A built-in's key in `Plugins::selected`, before its id.
pub const BUILTIN: &str = "builtin:";

impl<'a> Item<'a> {
    pub fn id(&self) -> &'a str {
        match self {
            Item::Builtin(b) => s(b, "id").unwrap_or_default(),
            Item::Listing(l) => &l.id,
        }
    }

    pub fn name(&self) -> &'a str {
        match self {
            Item::Builtin(b) => s(b, "name").unwrap_or_else(|| self.id()),
            Item::Listing(l) => &l.name,
        }
    }

    /// What `Plugins::selected` holds while its page is open.
    pub fn key(&self) -> String {
        match self {
            Item::Builtin(_) => format!("{BUILTIN}{}", self.id()),
            Item::Listing(l) => l.key.clone(),
        }
    }

    pub fn description(&self) -> &'a str {
        match self {
            Item::Builtin(b) => s(b, "description").unwrap_or_default(),
            Item::Listing(l) => l
                .description
                .as_deref()
                .or_else(|| l.catalog.as_ref().and_then(|c| s(c, "summary")))
                .unwrap_or_default(),
        }
    }

    pub fn tags(&self) -> Vec<Tag> {
        match self {
            Item::Builtin(b) => tags(b.get("tags")),
            Item::Listing(l) => l.tags.clone(),
        }
    }

    pub fn maker(&self) -> Maker {
        match self {
            Item::Builtin(_) => Maker::Vorn,
            Item::Listing(l) => Maker::of(l),
        }
    }

    fn matches(&self, query: &str) -> bool {
        match self {
            Item::Listing(l) => matches_query(l, query),
            Item::Builtin(_) => {
                let hay =
                    format!("{} {} {}", self.name(), self.description(), self.id()).to_lowercase();
                query
                    .to_lowercase()
                    .split_whitespace()
                    .all(|t| hay.contains(t))
            }
        }
    }
}

/// A heading of the Installed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Heading {
    Attention,
    Tag(Tag),
    Other,
}

impl Heading {
    pub fn label(self) -> &'static str {
        match self {
            Heading::Attention => "Needs attention",
            Heading::Tag(t) => t.plural(),
            Heading::Other => "Other",
        }
    }
}

/// One row of the Installed list and what it says on its right.
#[derive(Debug, Clone)]
pub struct Row<'a> {
    pub item: Item<'a>,
    pub status: Option<Status>,
}

/// Whether a listing is on this machine: a pack, or a connector with an account.
pub fn installed(l: &Listing) -> bool {
    l.pack.is_some()
        || (matches!(l.source, Source::Builtin | Source::Mcp) && (l.connected > 0 || l.implicit))
}

/// The accounts of connector `id`, implicit ones aside.
fn accounts<'a>(p: &'a Plugins, id: &'a str) -> impl Iterator<Item = &'a Value> {
    p.connections
        .iter()
        .filter(move |c| connection_connector(c) == id && !is_implicit(c))
}

/// What a listing's row says, `default_agent` being the extension the
/// default agent comes in.
pub fn status(p: &Plugins, l: &Listing, default_agent: Option<&str>) -> Option<Status> {
    let state = p.state(l);
    let said = describe(&state);
    if said.busy || p.busy(&l.id) {
        return Some(Status::Busy(said.label));
    }
    if matches!(state, PackState::Rejected(_)) {
        return Some(Status::Attention("Couldn't install".to_owned()));
    }
    let signed_out = p
        .status_of(&l.id)
        .is_some_and(|st| st.get("authed") == Some(&Value::Bool(false)));
    if signed_out && l.connected > 0 {
        return Some(Status::Attention("Sign in again".to_owned()));
    }
    let wants_account = l.tags.contains(&Tag::Connection) && l.pack.is_some();
    if wants_account && l.connected == 0 && !l.implicit {
        return Some(Status::Attention("Connect an account".to_owned()));
    }
    if matches!(
        state,
        PackState::Installed {
            available: Some(_),
            ..
        }
    ) {
        return Some(Status::Update);
    }
    let mut mine = accounts(p, &l.id);
    if let Some(first) = mine.next() {
        let name = s(first, "name")
            .filter(|n| !n.is_empty())
            .unwrap_or("Signed in");
        let more = mine.count();
        return Some(Status::Account(if more == 0 {
            name.to_owned()
        } else {
            format!("{name} +{more}")
        }));
    }
    if l.tags.contains(&Tag::Agent) {
        return Some(Status::Note(if default_agent == Some(l.id.as_str()) {
            "Default"
        } else {
            "On"
        }));
    }
    if l.implicit {
        return Some(Status::Note("Ready"));
    }
    Some(Status::Note("On"))
}

/// The Installed list under `query`: what needs the person first, then each
/// kind by its first tag, then the rest, each by name.
pub fn sections<'a>(
    p: &'a Plugins,
    builtins: &'a [Value],
    query: &str,
    default_agent: Option<&str>,
) -> Vec<(Heading, Vec<Row<'a>>)> {
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<(Heading, Row<'a>)> = Vec::new();
    for b in builtins {
        let item = Item::Builtin(b);
        if !seen.insert(item.id()) || !item.matches(query) {
            continue;
        }
        let on = b.get("on") != Some(&Value::Bool(false));
        let heading = item
            .tags()
            .first()
            .map_or(Heading::Other, |t| Heading::Tag(*t));
        let status = Status::Note(if on { "On" } else { "Off" });
        rows.push((
            heading,
            Row {
                item,
                status: Some(status),
            },
        ));
    }
    for l in p.listings.iter().filter(|l| installed(l)) {
        let item = Item::Listing(l);
        if !seen.insert(l.id.as_str()) || !item.matches(query) {
            continue;
        }
        let status = status(p, l, default_agent);
        let heading = match (&status, l.tags.first()) {
            (Some(Status::Attention(_)), _) => Heading::Attention,
            (_, Some(t)) => Heading::Tag(*t),
            (_, None) => Heading::Other,
        };
        rows.push((heading, Row { item, status }));
    }
    rows.sort_by(|(ha, a), (hb, b)| {
        ha.cmp(hb).then_with(|| {
            a.item
                .name()
                .to_lowercase()
                .cmp(&b.item.name().to_lowercase())
        })
    });
    let mut out: Vec<(Heading, Vec<Row<'a>>)> = Vec::new();
    for (h, row) in rows {
        match out.last_mut() {
            Some((last, items)) if *last == h => items.push(row),
            _ => out.push((h, vec![row])),
        }
    }
    out
}

/// How many rows the Installed list has with no search.
pub fn installed_count(p: &Plugins, builtins: &[Value]) -> usize {
    sections(p, builtins, "", None)
        .iter()
        .map(|(_, rows)| rows.len())
        .sum()
}

/// How many extensions Vorn knows of, installed or not, each once.
pub fn total(p: &Plugins, builtins: &[Value]) -> usize {
    let mut ids: std::collections::HashSet<&str> =
        builtins.iter().filter_map(|b| s(b, "id")).collect();
    ids.extend(p.listings.iter().map(|l| l.id.as_str()));
    ids.len()
}

/// What a Browse tile's button says and does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    Install,
    /// A built-in or MCP connector: add an account.
    Connect,
    Installed,
    /// Installing now, with what it is doing.
    Busy(String),
    /// Nothing to do yet.
    None,
}

impl Verb {
    pub fn of(p: &Plugins, l: &Listing) -> Verb {
        let state = p.state(l);
        let said = describe(&state);
        if said.busy {
            return Verb::Busy(said.label);
        }
        if installed(l) {
            return Verb::Installed;
        }
        if matches!(l.source, Source::Builtin | Source::Mcp) && l.can_add(&state) {
            return Verb::Connect;
        }
        if l.installable() && said.action.is_some() {
            return Verb::Install;
        }
        Verb::None
    }
}

/// Browse's tiles: everything that can be added, under `query` and `tag`, by name.
pub fn browse<'a>(p: &'a Plugins, query: &str, tag: Option<Tag>) -> Vec<&'a Listing> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<&Listing> = p
        .listings
        .iter()
        .filter(|l| l.source != Source::Installed)
        .filter(|l| matches_query(l, query))
        .filter(|l| tag.is_none_or(|t| l.tags.contains(&t)))
        .filter(|l| seen.insert(l.id.as_str()))
        .collect();
    out.sort_by_key(|l| l.name.to_lowercase());
    out
}

/// The tags Browse offers a chip for: those some listing carries.
pub fn browse_tags(p: &Plugins) -> Vec<Tag> {
    Tag::ALL
        .into_iter()
        .filter(|t| p.listings.iter().any(|l| l.tags.contains(t)))
        .collect()
}

/// The installed packs with a newer release, by name.
pub fn updates(p: &Plugins) -> Vec<&Listing> {
    let mut out = pack_updates(&p.listings).1;
    out.sort_by_key(|l| l.name.to_lowercase());
    out
}

/// A colour index for a monogram, steady for an id.
pub fn monogram_hue(id: &str, hues: usize) -> usize {
    let h = id.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    h as usize % hues.max(1)
}

/// A monogram's letter: the name's first letter or digit, upper case.
pub fn monogram(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn plugins(catalog: Value, packs: Value, connections: Value) -> Plugins {
        let mut p = Plugins {
            catalog: catalog.as_array().cloned().unwrap_or_default(),
            packs: packs.as_array().cloned().unwrap_or_default(),
            connections: connections.as_array().cloned().unwrap_or_default(),
            ..Plugins::default()
        };
        p.rebuild();
        p
    }

    fn names<'a>(rows: &'a [Row<'a>]) -> Vec<&'a str> {
        rows.iter().map(|r| r.item.name()).collect()
    }

    const GIT: &str = "https://github.com/vorn-run/extensions.git";

    #[test]
    fn tags_read_in_order_and_drop_unknown_ones() {
        let t = tags(Some(&json!(["tools", "pane", "made-up", "pane"])));
        assert_eq!(t, [Tag::Pane, Tag::Tools]);
        assert_eq!(Tag::Steps.label(), "Workflow steps");
        assert_eq!(Tag::Tools.label(), "Agent tools");
    }

    #[test]
    fn what_needs_you_rises_above_the_kinds() {
        let p = plugins(
            json!([
                { "id": "rss", "name": "RSS", "kind": "extension", "tags": ["connection"], "git": GIT, "version": "1.0.0" },
                { "id": "claude", "name": "Claude Code", "kind": "extension", "tags": ["agent"], "git": GIT, "version": "2.0.0" },
                { "id": "codex", "name": "Codex", "kind": "extension", "tags": ["agent"], "git": GIT, "version": "1.0.0" },
            ]),
            json!([
                { "id": "rss", "name": "RSS", "version": "1.0.0", "tags": ["connection"] },
                { "id": "claude", "name": "Claude Code", "version": "1.0.0", "tags": ["agent"] },
                { "id": "codex", "name": "Codex", "version": "1.0.0", "tags": ["agent"] },
                { "id": "notes", "name": "Notes", "version": "1.0.0" },
            ]),
            json!([]),
        );
        let builtins =
            [json!({ "id": "vorn.tasks", "name": "Tasks", "on": false, "tags": ["pane"] })];
        let got = sections(&p, &builtins, "", Some("codex"));
        let heads: Vec<Heading> = got.iter().map(|(h, _)| *h).collect();
        assert_eq!(
            heads,
            [
                Heading::Attention,
                Heading::Tag(Tag::Agent),
                Heading::Tag(Tag::Pane),
                Heading::Other
            ]
        );
        assert_eq!(names(&got[0].1), ["RSS"]);
        assert_eq!(
            got[0].1[0].status,
            Some(Status::Attention("Connect an account".into()))
        );
        assert_eq!(names(&got[1].1), ["Claude Code", "Codex"]);
        assert_eq!(got[1].1[0].status, Some(Status::Update));
        assert_eq!(got[1].1[1].status, Some(Status::Note("Default")));
        assert_eq!(got[2].1[0].status, Some(Status::Note("Off")));
        assert_eq!(installed_count(&p, &builtins), 5);
        assert_eq!(updates(&p).len(), 1);
    }

    #[test]
    fn a_connection_with_an_account_names_it() {
        let p = plugins(
            json!([]),
            json!([{ "id": "gh", "name": "GitHub", "version": "1.0.0", "tags": ["connection"] }]),
            json!([
                { "id": "c1", "connectorId": "gh", "name": "vorn-run" },
                { "id": "c2", "connectorId": "gh", "name": "me" },
            ]),
        );
        let got = sections(&p, &[], "", None);
        assert_eq!(got[0].0, Heading::Tag(Tag::Connection));
        assert_eq!(
            got[0].1[0].status,
            Some(Status::Account("vorn-run +1".into()))
        );
    }

    #[test]
    fn search_narrows_the_installed_list() {
        let p = plugins(
            json!([]),
            json!([
                { "id": "a", "name": "Alpha", "version": "1.0.0" },
                { "id": "b", "name": "Beta", "version": "1.0.0" },
            ]),
            json!([]),
        );
        let got = sections(&p, &[], "bet", None);
        assert_eq!(names(&got[0].1), ["Beta"]);
    }

    #[test]
    fn browse_filters_by_tag_and_says_what_each_tile_does() {
        let p = plugins(
            json!([
                { "id": "gemini", "name": "Gemini CLI", "kind": "extension", "tags": ["agent"], "git": GIT, "version": "1.0.0" },
                { "id": "devices", "name": "Devices", "kind": "extension", "tags": ["pane", "tools"], "git": GIT, "version": "1.0.0" },
            ]),
            json!([{ "id": "devices", "name": "Devices", "version": "1.0.0", "tags": ["pane", "tools"] }]),
            json!([]),
        );
        let agents = browse(&p, "", Some(Tag::Agent));
        let names: Vec<&str> = agents.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Gemini CLI"]);
        assert_eq!(Verb::of(&p, agents[0]), Verb::Install);
        let all = browse(&p, "", None);
        assert_eq!(all.len(), 2);
        assert_eq!(Verb::of(&p, all[0]), Verb::Installed);
        assert_eq!(browse_tags(&p), [Tag::Agent, Tag::Pane, Tag::Tools]);
    }

    #[test]
    fn makers_and_monograms() {
        let p = plugins(
            json!([]),
            json!([
                { "id": "mine", "name": "mine", "version": "1.0.0", "linked": true },
                { "id": "acme", "name": "Acme", "version": "1.0.0", "publisher": "Acme Co" },
                { "id": "ours", "name": "Ours", "version": "1.0.0", "publisher": "Vorn" },
                { "id": "theirs", "name": "Theirs", "version": "1.0.0",
                  "source": { "git": "https://github.com/someone/theirs.git" } },
            ]),
            json!([]),
        );
        let maker = |id| Maker::of(p.listing_by_id(id).unwrap()).label();
        assert_eq!(maker("mine"), "By you");
        assert_eq!(maker("acme"), "By Acme Co");
        assert_eq!(maker("ours"), "By Vorn");
        assert_eq!(maker("theirs"), "By someone/theirs");
        assert_eq!(monogram("  rss feeds"), "R");
        assert_eq!(monogram_hue("rss", 4), monogram_hue("rss", 4));
        assert!(monogram_hue("anything", 4) < 4);
    }
}
