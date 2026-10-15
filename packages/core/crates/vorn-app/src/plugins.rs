//! Settings › Plugins: the connectors a person can add, the packs on disk and
//! the connections made to them. The pure half lives here (listings, pack
//! status, filters, form rules) so the screen and the RPC glue only read it.

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use crate::library::connection_connector;
use crate::runs::s;
use crate::templates::Fix;

/// Every control the page can fire.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlugAct {
    /// Reload everything the page reads.
    Refresh,
    Tab(Tab),
    /// Open a listing's detail, or close it.
    Select(Option<String>),
    /// Open the add form for a listing key.
    Add(String),
    CloseForm,
    /// Install, update or retry a listing's pack.
    Install(String),
    /// Pick a `.vorn.tgz` on this disk, then check it.
    InstallFile,
    ConfirmInstall,
    CancelInstall,
    Remove(String),
    Rollback(String),
    CatalogRefresh,
    Text(Field),
    Pick(Pick),
    /// Switch the MCP form between a package and a manual launch line.
    McpManual(bool),
    Probe,
    UseToken,
    Save,
    Backfill(String),
    Test(String),
    Delete(String),
    RunWorkflow(String, String),
    Reseed(String, String),
    SignIn(String),
    SignOut(String),
    OpenUrl(String),
    Use,
    /// Open a seeded workflow in the editor.
    OpenWorkflow(String),
    /// A workflow requirement's fix, applied once the page has loaded.
    Fix(Fix),
}

/// The id a page picker draws under, so its menu's choices can be told apart.
pub fn pick_id(pick: &Pick) -> crate::ui::Id {
    crate::ui::Id::new("plug-pick").child(pick)
}

/// The id a page text field edits under.
pub fn field_id(field: &Field) -> crate::ui::Id {
    crate::ui::Id::new("plug-field").child(field)
}

/// The three views the page offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    #[default]
    Installed,
    Connections,
    Browse,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Installed, Tab::Connections, Tab::Browse];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Installed => "Installed",
            Tab::Connections => "Connections",
            Tab::Browse => "Browse",
        }
    }
}

/// A text field the page binds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Field {
    Query,
    /// A built-in's auth or filter key.
    Auth(String),
    Owner,
    Repo,
    /// The package or launch line the SDK form looks up.
    Spec,
    /// A pack's env variable.
    Env(String),
    ProfileName,
    ProfileUrl,
    ProfileHeader,
    ProfileSecret,
}

/// A picker the page binds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pick {
    Category,
    Kind,
    Rung,
    Project,
    Trigger,
    /// A built-in's select field.
    Select(String),
}

/// Where a listing came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Catalog,
    Installed,
    Mcp,
}

/// Whether a listing polls a service or adds to a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Connector,
    Extension,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Connector, Kind::Extension];

    fn parse(v: Option<&str>) -> Option<Kind> {
        match v? {
            "extension" => Some(Kind::Extension),
            "connector" => Some(Kind::Connector),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Connector => "Connectors",
            Kind::Extension => "Extensions",
        }
    }
}

/// How a connector signs in, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rung {
    None,
    Cli,
    Key,
    Browser,
    Oauth,
}

impl Rung {
    pub const ALL: [Rung; 5] = [Rung::None, Rung::Cli, Rung::Key, Rung::Browser, Rung::Oauth];

    pub fn parse(v: Option<&str>) -> Option<Rung> {
        Some(match v? {
            "none" => Rung::None,
            "cli" => Rung::Cli,
            "key" => Rung::Key,
            "browser" => Rung::Browser,
            "oauth" => Rung::Oauth,
            _ => return None,
        })
    }

    /// The filter label.
    pub fn label(self) -> &'static str {
        match self {
            Rung::None => "Needs no sign-in",
            Rung::Cli => "Signs in with a CLI",
            Rung::Key => "Needs a key",
            Rung::Browser => "Signs in through the browser",
            Rung::Oauth => "Signs in with OAuth",
        }
    }

    /// The row badge.
    pub fn badge(self) -> &'static str {
        match self {
            Rung::None => "no sign-in",
            Rung::Cli => "CLI login",
            Rung::Key => "key",
            Rung::Browser => "browser",
            Rung::Oauth => "OAuth",
        }
    }

    /// The detail page's sentence.
    pub fn detail(self) -> &'static str {
        match self {
            Rung::None => "Nothing — ready as soon as it is installed.",
            other => other.label(),
        }
    }
}

/// What a status means, and so how it is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Blocked,
    Broken,
    Live,
    Settled,
    Idle,
}

/// One install's progress as vornd reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub phase: String,
    pub percent: Option<f64>,
    pub error: Option<String>,
}

impl Progress {
    pub fn phase(phase: &str) -> Self {
        Progress {
            phase: phase.to_owned(),
            percent: None,
            error: None,
        }
    }

    pub fn from_value(v: &Value) -> Self {
        Progress {
            phase: s(v, "phase").unwrap_or("checking").to_owned(),
            percent: v.get("percent").and_then(Value::as_f64),
            error: s(v, "error").map(str::to_owned),
        }
    }

    fn terminal(&self) -> bool {
        matches!(self.phase.as_str(), "installed" | "failed")
    }
}

/// `PackState`: a live install outranks a rejection, which outranks the disk.
#[derive(Debug, Clone, PartialEq)]
pub enum PackState {
    Absent,
    NotReleased,
    Installing {
        phase: String,
        percent: Option<f64>,
    },
    Installed {
        version: String,
        available: Option<String>,
        previous: Option<String>,
    },
    Rejected(String),
}

/// What a row offers to do with its pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackAction {
    Install,
    Update,
    Retry,
}

impl PackAction {
    pub fn label(self) -> &'static str {
        match self {
            PackAction::Install => "Install",
            PackAction::Update => "Update",
            PackAction::Retry => "Retry",
        }
    }
}

/// `PackStatusView`.
#[derive(Debug, Clone, PartialEq)]
pub struct PackStatus {
    pub label: String,
    pub detail: Option<String>,
    pub tone: Tone,
    pub percent: Option<f64>,
    pub action: Option<PackAction>,
    pub busy: bool,
}

/// `packStateFor`.
pub fn pack_state(
    pack: Option<&Value>,
    catalog: Option<&Value>,
    progress: Option<&Progress>,
) -> PackState {
    let released = catalog.is_none_or(|c| s(c, "packUrl").is_some_and(|u| !u.is_empty()));
    if let Some(p) = progress {
        if !p.terminal() {
            return PackState::Installing {
                phase: p.phase.clone(),
                percent: p.percent,
            };
        }
        if p.phase == "failed" {
            let error = p.error.clone();
            return PackState::Rejected(
                error.unwrap_or_else(|| "The pack could not be installed".to_owned()),
            );
        }
    }
    let Some(pack) = pack else {
        return if released {
            PackState::Absent
        } else {
            PackState::NotReleased
        };
    };
    let version = s(pack, "version").unwrap_or("").to_owned();
    let published = if released {
        catalog.and_then(|c| s(c, "version"))
    } else {
        None
    };
    PackState::Installed {
        available: published
            .filter(|p| is_newer(p, &version))
            .map(str::to_owned),
        previous: s(pack, "previousVersion").map(str::to_owned),
        version,
    }
}

/// `describePackStatus`.
pub fn describe(state: &PackState) -> PackStatus {
    let view = |label: &str, detail: Option<String>, tone, action| PackStatus {
        label: label.to_owned(),
        detail,
        tone,
        percent: None,
        action,
        busy: false,
    };
    match state {
        PackState::Absent => view("Install", None, Tone::Idle, Some(PackAction::Install)),
        PackState::NotReleased => view(
            "Not released yet",
            Some("Not released yet, so there is nothing to install.".to_owned()),
            Tone::Idle,
            None,
        ),
        PackState::Installing { phase, percent } => PackStatus {
            label: match phase.as_str() {
                "downloading" => "Downloading",
                "verifying" => "Verifying",
                "installing" => "Installing",
                "installed" => "Installed",
                "failed" => "Failed",
                _ => "Checking",
            }
            .to_owned(),
            detail: None,
            tone: Tone::Live,
            percent: if phase == "downloading" {
                *percent
            } else {
                None
            },
            action: None,
            busy: true,
        },
        PackState::Installed {
            version,
            available: Some(next),
            ..
        } => view(
            "Update",
            Some(format!("v{version} → {next} available")),
            Tone::Blocked,
            Some(PackAction::Update),
        ),
        PackState::Installed { version, .. } => view(
            "Installed",
            Some(format!("v{version}")),
            Tone::Settled,
            None,
        ),
        PackState::Rejected(error) => view(
            "Couldn't install",
            Some(error.clone()),
            Tone::Broken,
            Some(PackAction::Retry),
        ),
    }
}

/// `isNewerVersion`: semver precedence, prereleases included.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    #[derive(PartialEq)]
    enum Part<'a> {
        Num(u64),
        Text(&'a str),
    }
    fn split(v: &str) -> (Vec<u64>, Vec<Part<'_>>) {
        let v = v.split('+').next().unwrap_or("");
        let (release, pre) = v.split_once('-').unwrap_or((v, ""));
        let release = release.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        let pre = pre
            .split('.')
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().map_or(Part::Text(p), Part::Num))
            .collect();
        (release, pre)
    }
    let (l, lp) = split(candidate);
    let (r, rp) = split(current);
    for i in 0..l.len().max(r.len()) {
        let (a, b) = (
            l.get(i).copied().unwrap_or(0),
            r.get(i).copied().unwrap_or(0),
        );
        if a != b {
            return a > b;
        }
    }
    if lp.is_empty() {
        return !rp.is_empty();
    }
    if rp.is_empty() {
        return false;
    }
    for i in 0..lp.len().max(rp.len()) {
        match (lp.get(i), rp.get(i)) {
            (a, b) if a == b => continue,
            (None, _) => return false,
            (_, None) => return true,
            (Some(Part::Num(a)), Some(Part::Num(b))) => return a > b,
            (Some(Part::Num(_)), _) => return false,
            (_, Some(Part::Num(_))) => return true,
            (Some(Part::Text(a)), Some(Part::Text(b))) => return a > b,
        }
    }
    false
}

/// One row in the directory, whatever it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub key: String,
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub capabilities: Vec<String>,
    pub category: String,
    pub kind: Kind,
    pub source: Source,
    pub keywords: Vec<String>,
    pub connected: usize,
    pub implicit: bool,
    pub catalog: Option<Value>,
    pub pack: Option<Value>,
    pub mcp: Option<Value>,
    /// The `{viewBox, paths}` glyph a pack or catalog entry draws.
    pub icon: Option<Value>,
    pub rung: Option<Rung>,
    pub verified: Option<Value>,
    pub contributes: Option<Value>,
    pub permissions: Vec<String>,
    pub activates: Option<Value>,
}

impl Listing {
    /// Usable now: connected, or ready without asking.
    pub fn ready(&self) -> bool {
        self.connected > 0 || self.implicit
    }

    pub fn state(&self, progress: Option<&Progress>) -> PackState {
        pack_state(self.pack.as_ref(), self.catalog.as_ref(), progress)
    }

    /// `canAddConnection`.
    pub fn can_add(&self, state: &PackState) -> bool {
        if self.kind == Kind::Extension {
            return false;
        }
        match (self.source, state) {
            (Source::Builtin | Source::Mcp, _) | (_, PackState::Installed { .. }) => true,
            (_, PackState::NotReleased) => false,
            _ => self
                .catalog
                .as_ref()
                .is_some_and(|c| s(c, "packageName").is_some()),
        }
    }

    /// Whether the row is a pack that installs at all.
    pub fn installable(&self) -> bool {
        matches!(self.source, Source::Catalog | Source::Installed)
    }

    pub fn version(&self) -> Option<&str> {
        self.catalog.as_ref().and_then(|c| s(c, "version"))
    }

    pub fn has_actions(&self) -> bool {
        self.capabilities.iter().any(|c| c == "actions")
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn array(v: &Value, key: &str) -> usize {
    v.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

/// `isImplicitConnection`.
pub fn is_implicit(conn: &Value) -> bool {
    conn.pointer("/filters/implicit")
        .is_some_and(|v| v == true || v == "true")
}

/// `buildConnectorListings`, sorted ready-first then by name.
pub fn build_listings(
    builtins: &[Value],
    catalog: &[Value],
    connections: &[Value],
    packs: &[Value],
    mcp: &[Value],
) -> Vec<Listing> {
    let mut owned: HashMap<&str, (usize, bool)> = HashMap::new();
    for conn in connections {
        let entry = owned.entry(connection_connector(conn)).or_default();
        if is_implicit(conn) {
            entry.1 = true;
        } else {
            entry.0 += 1;
        }
    }
    let owned_of = |id: &str| owned.get(id).copied().unwrap_or_default();
    let pack_of = |id: &str| packs.iter().find(|p| s(p, "id") == Some(id));
    let base = |id: &str, name: &str, category: &str, source| {
        let (connected, implicit) = owned_of(id);
        Listing {
            key: id.to_owned(),
            id: id.to_owned(),
            name: name.to_owned(),
            description: None,
            capabilities: Vec::new(),
            category: category.to_owned(),
            kind: Kind::Connector,
            source,
            keywords: Vec::new(),
            connected,
            implicit,
            catalog: None,
            pack: None,
            mcp: None,
            icon: None,
            rung: None,
            verified: None,
            contributes: None,
            permissions: Vec::new(),
            activates: None,
        }
    };
    let with_pack = |l: &mut Listing, pack: &Value| {
        if let Some(r) = Rung::parse(pack.pointer("/auth/rung").and_then(Value::as_str)) {
            l.rung = Some(r);
        }
        if let Some(c) = pack.get("contributes") {
            l.contributes = Some(c.clone());
        }
        if pack.get("permissions").is_some() {
            l.permissions = strings(pack.get("permissions"));
        }
        if let Some(a) = pack.get("activates") {
            l.activates = Some(a.clone());
        }
        l.pack = Some(pack.clone());
    };
    let mut out = Vec::new();
    for c in builtins
        .iter()
        .filter(|c| c.get("addable") != Some(&Value::Bool(false)))
    {
        let id = s(c, "id").unwrap_or("");
        let mut l = base(id, s(c, "name").unwrap_or(id), "Built in", Source::Builtin);
        l.capabilities = strings(c.get("capabilities"));
        out.push(l);
    }
    for entry in catalog {
        let id = s(entry, "id").unwrap_or("");
        let pack = pack_of(id);
        let category = s(entry, "category").unwrap_or("Other");
        let mut l = base(
            id,
            s(entry, "name").unwrap_or(id),
            category,
            Source::Catalog,
        );
        l.key = format!("catalog:{id}");
        l.description = s(entry, "description").map(str::to_owned);
        l.capabilities = strings(entry.get("capabilities"));
        l.keywords = strings(entry.get("keywords"));
        l.kind = pack
            .and_then(|p| Kind::parse(s(p, "kind")))
            .or_else(|| Kind::parse(s(entry, "kind")))
            .unwrap_or(Kind::Connector);
        l.icon = entry.get("icon").cloned();
        l.rung = Rung::parse(s(entry, "authRung"));
        l.verified = entry.get("verified").cloned();
        l.contributes = entry.get("contributes").cloned();
        l.permissions = strings(entry.get("permissions"));
        l.activates = entry.get("activates").cloned();
        l.catalog = Some(entry.clone());
        if let Some(pack) = pack {
            with_pack(&mut l, pack);
        }
        out.push(l);
    }
    let known = |id: &str| {
        catalog.iter().any(|e| s(e, "id") == Some(id))
            || builtins.iter().any(|b| s(b, "id") == Some(id))
    };
    for pack in packs.iter().filter(|p| !known(s(p, "id").unwrap_or(""))) {
        let id = s(pack, "id").unwrap_or("");
        let mut l = base(
            id,
            s(pack, "name").unwrap_or(id),
            "Installed",
            Source::Installed,
        );
        l.key = format!("installed:{id}");
        l.description = s(pack, "description").map(str::to_owned);
        if array(pack, "triggers") > 0 {
            l.capabilities.push("triggers".to_owned());
        }
        if array(pack, "actions") > 0 {
            l.capabilities.push("actions".to_owned());
        }
        l.kind = Kind::parse(s(pack, "kind")).unwrap_or(Kind::Connector);
        l.icon = pack.get("icon").cloned();
        with_pack(&mut l, pack);
        out.push(l);
    }
    for server in mcp {
        let id = s(server, "id").unwrap_or("");
        let category = s(server, "category").unwrap_or("MCP servers");
        let mut l = base(id, s(server, "name").unwrap_or(id), category, Source::Mcp);
        l.key = format!("mcp:{id}");
        l.description = s(server, "description").map(str::to_owned);
        l.capabilities = vec!["actions".to_owned()];
        l.keywords = strings(server.get("keywords"));
        l.mcp = Some(server.clone());
        out.push(l);
    }
    out.sort_by(|a, b| {
        b.ready()
            .cmp(&a.ready())
            .then_with(|| compare_names(&a.name, &b.name))
    });
    out
}

/// Close to `localeCompare`: case folds first, then the raw text breaks ties.
fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// `EXTENSION_PERMISSION`: the bucket and phrase a permission reads as.
pub fn permission(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "git.read" => ("Reads", "the worktree's diff and status"),
        "terminal.read" => ("Reads", "the session's recent terminal output"),
        "terminal.selection" => ("Reads", "the text selected in the terminal"),
        "agent.usage" => ("Reads", "the agent's context and provider allowance"),
        "agent.permissions" => ("Answers", "the agents' requests to use a tool"),
        "terminal.send" => ("Sends", "text into the session's terminal"),
        "card.rename" => ("Renames", "the session card"),
        _ => return None,
    })
}

const PERMISSIONS: [&str; 6] = [
    "git.read",
    "terminal.read",
    "terminal.selection",
    "agent.usage",
    "terminal.send",
    "card.rename",
];

/// `permissionRows`: what an extension asks for, under its verb.
pub fn permission_rows(asked: &[String]) -> Vec<(&'static str, Vec<&'static str>)> {
    ["Reads", "Sends", "Renames"]
        .into_iter()
        .map(|bucket| {
            let items = PERMISSIONS
                .iter()
                .filter(|p| asked.iter().any(|a| a == *p))
                .filter_map(|p| permission(p))
                .filter(|(b, _)| *b == bucket)
                .map(|(_, phrase)| phrase)
                .collect::<Vec<_>>();
            (bucket, items)
        })
        .filter(|(_, items)| !items.is_empty())
        .collect()
}

/// `notAsked`.
pub fn not_asked(asked: &[String]) -> Vec<&'static str> {
    PERMISSIONS
        .iter()
        .filter(|p| !asked.iter().any(|a| a == *p))
        .filter_map(|p| permission(p).map(|(_, phrase)| phrase))
        .collect()
}

/// `count`: "1 pane", "2 panes".
pub fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// `describeContributions`: "2 panes, 1 footer".
pub fn describe_contributions(contributes: Option<&Value>) -> String {
    let n = |key| contributes.map_or(0, |c| array(c, key));
    [
        (n("panes"), "pane"),
        (n("footers"), "footer"),
        (n("linkHandlers"), "link handler"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, noun)| count(n, noun))
    .collect::<Vec<_>>()
    .join(", ")
}

fn any_of(values: &[String]) -> String {
    match values {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// `describeActivation`: where an extension shows, as clauses.
pub fn describe_activation(activates: Option<&Value>) -> Vec<String> {
    let list = |key| {
        strings(activates.and_then(|a| a.get(key)))
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
    };
    let mut clauses = Vec::new();
    let contains = list("workspaceContains");
    if !contains.is_empty() {
        clauses.push(format!("projects with {}", any_of(&contains)));
    }
    let hosts: Vec<String> = list("remoteHost")
        .into_iter()
        .map(|h| {
            match h.as_str() {
                "github.com" => "GitHub",
                "gitlab.com" => "GitLab",
                "bitbucket.org" => "Bitbucket",
                _ => return h,
            }
            .to_owned()
        })
        .collect();
    if !hosts.is_empty() {
        clauses.push(format!("{} remotes", any_of(&hosts)));
    }
    let agents: Vec<String> = list("agent")
        .into_iter()
        .map(|a| match crate::onboarding::agent_def(&a) {
            _ if a == "shell" => a,
            Some(def) => def.name.to_owned(),
            None => a,
        })
        .collect();
    if !agents.is_empty() {
        clauses.push(format!("{} sessions", any_of(&agents)));
    }
    let platforms: Vec<String> = list("platform")
        .into_iter()
        .map(|p| {
            match p.as_str() {
                "darwin" => "macOS",
                "linux" => "Linux",
                "win32" => "Windows",
                _ => return p,
            }
            .to_owned()
        })
        .collect();
    if !platforms.is_empty() {
        clauses.push(any_of(&platforms));
    }
    if clauses.is_empty() {
        clauses.push("every session".to_owned());
    }
    clauses
}

/// The terms a query matches against.
fn haystack(l: &Listing) -> String {
    let mut parts = vec![
        l.name.as_str(),
        l.description.as_deref().unwrap_or(""),
        &l.category,
    ];
    parts.extend(l.capabilities.iter().map(String::as_str));
    parts.extend(l.keywords.iter().map(String::as_str));
    for key in ["panes", "footers", "linkHandlers"] {
        if let Some(items) = l.contributes.as_ref().and_then(|c| c.get(key)?.as_array()) {
            parts.extend(items.iter().filter_map(|i| s(i, "title")));
        }
    }
    parts.extend(
        l.permissions
            .iter()
            .filter_map(|p| permission(p).map(|(_, phrase)| phrase)),
    );
    parts.join(" ").to_lowercase()
}

/// `filterConnectorListings`: every term must match.
pub fn matches_query(l: &Listing, query: &str) -> bool {
    let q = query.to_lowercase();
    let terms: Vec<&str> = q.split_whitespace().collect();
    if terms.is_empty() {
        return true;
    }
    let hay = haystack(l);
    terms.iter().all(|t| hay.contains(t))
}

pub const CALLABLE: &str = "Can be called from a step";
pub const CONNECTED: &str = "Connected";

/// `filterByCategory`.
pub fn matches_category(l: &Listing, category: Option<&str>) -> bool {
    match category {
        None => true,
        Some(CALLABLE) => l.has_actions(),
        Some(CONNECTED) => l.connected > 0,
        Some(c) => l.category == c,
    }
}

/// `connectorCategories`: sorted categories, then the two facets present.
pub fn categories(listings: &[Listing]) -> Vec<String> {
    let mut out: Vec<String> = listings.iter().map(|l| l.category.clone()).collect();
    out.sort();
    out.dedup();
    if listings.iter().any(Listing::has_actions) {
        out.push(CALLABLE.to_owned());
    }
    if listings.iter().any(|l| l.connected > 0) {
        out.push(CONNECTED.to_owned());
    }
    out
}

/// `connectorKinds`.
pub fn kinds(listings: &[Listing]) -> Vec<Kind> {
    Kind::ALL
        .into_iter()
        .filter(|k| listings.iter().any(|l| l.kind == *k))
        .collect()
}

/// `connectorAuthRungs`.
pub fn rungs(listings: &[Listing]) -> Vec<Rung> {
    Rung::ALL
        .into_iter()
        .filter(|r| listings.iter().any(|l| l.rung == Some(*r)))
        .collect()
}

/// `groupListingsByCategory`: first-seen order.
pub fn sections(listings: Vec<&Listing>) -> Vec<(String, Vec<&Listing>)> {
    let mut out: Vec<(String, Vec<&Listing>)> = Vec::new();
    for l in listings {
        match out.iter_mut().find(|(c, _)| *c == l.category) {
            Some((_, items)) => items.push(l),
            None => out.push((l.category.clone(), vec![l])),
        }
    }
    out
}

/// `describeCatalogAge`.
pub fn catalog_age(fetched_at: Option<f64>, now: f64) -> String {
    let Some(at) = fetched_at else {
        return "Showing the connectors that shipped with Vorn".to_owned();
    };
    let minutes = ((now - at) / 60_000.0).floor().max(0.0) as usize;
    match minutes {
        0 => "Updated just now".to_owned(),
        1..=59 => format!("Updated {} ago", count(minutes, "minute")),
        _ if minutes / 60 < 24 => format!("Updated {} ago", count(minutes / 60, "hour")),
        _ => format!("Updated {} ago", count(minutes / 1440, "day")),
    }
}

/// `ConnectorDetails`: what a connector triggers on, offers and asks for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    pub triggers: Vec<(String, Option<String>)>,
    pub actions: Vec<(String, Option<String>)>,
    pub settings: Vec<(String, bool, Option<String>)>,
    pub known: bool,
}

fn labelled(items: Option<&Value>) -> Vec<(String, Option<String>)> {
    items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|i| {
            let label = s(i, "label")
                .or_else(|| s(i, "type"))
                .unwrap_or("")
                .to_owned();
            (
                label,
                s(i, "description")
                    .filter(|d| !d.is_empty())
                    .map(str::to_owned),
            )
        })
        .collect()
}

fn env_settings(items: Option<&Value>) -> Vec<(String, bool, Option<String>)> {
    items
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| {
            (
                s(e, "name").unwrap_or("").to_owned(),
                e.get("required") == Some(&Value::Bool(true)),
                s(e, "description")
                    .filter(|d| !d.is_empty())
                    .map(str::to_owned),
            )
        })
        .collect()
}

/// `listingDetails`: the pack beats the catalog, which beats nothing.
pub fn details(l: &Listing, builtins: &[Value]) -> Details {
    if let Some(pack) = &l.pack {
        return Details {
            triggers: labelled(pack.get("triggers")),
            actions: labelled(pack.get("actions")),
            settings: env_settings(pack.get("env")),
            known: true,
        };
    }
    if l.source == Source::Catalog {
        let entry = l.catalog.as_ref();
        let has = |key| entry.is_some_and(|e| e.get(key).is_some());
        if !has("triggers") && !has("actions") {
            let known = l.kind == Kind::Extension || l.contributes.is_some();
            return Details {
                known,
                ..Details::default()
            };
        }
        return Details {
            triggers: labelled(entry.and_then(|e| e.get("triggers"))),
            actions: labelled(entry.and_then(|e| e.get("actions"))),
            settings: env_settings(entry.and_then(|e| e.get("env"))),
            known: true,
        };
    }
    let Some(manifest) = builtins
        .iter()
        .find(|b| s(b, "id") == Some(l.id.as_str()))
        .and_then(|b| b.get("manifest"))
    else {
        return Details::default();
    };
    Details {
        triggers: labelled(manifest.get("triggers")),
        actions: labelled(manifest.get("actions")),
        settings: manifest
            .get("auth")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|f| {
                let name = s(f, "label").or_else(|| s(f, "key")).unwrap_or("");
                (
                    name.to_owned(),
                    f.get("required") == Some(&Value::Bool(true)),
                    None,
                )
            })
            .collect(),
        known: true,
    }
}

/// `matchesListing`: the checked pack is exactly what the row described.
pub fn matches_listing(preview: &Value, l: &Listing) -> bool {
    let version = s(preview, "version");
    s(preview, "id") == Some(l.id.as_str())
        && version.is_some()
        && version == l.version()
        && Rung::parse(preview.pointer("/auth/rung").and_then(Value::as_str))
            .is_some_and(|r| Some(r) == l.rung)
        && l.verified.as_ref().and_then(|v| s(v, "version")) == version
        && preview.get("installedVersion").is_none_or(Value::is_null)
}

/// Connections grouped under their connector, implicit ones left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Group<'a> {
    pub connector: String,
    pub name: String,
    pub version: Option<String>,
    pub listing: Option<&'a Listing>,
    pub connections: Vec<&'a Value>,
}

/// `groupConnections`, in server order.
pub fn group_connections<'a>(connections: &'a [Value], listings: &'a [Listing]) -> Vec<Group<'a>> {
    let mut out: Vec<Group<'a>> = Vec::new();
    for conn in connections.iter().filter(|c| !is_implicit(c)) {
        let id = connection_connector(conn);
        if let Some(g) = out.iter_mut().find(|g| g.connector == id) {
            g.connections.push(conn);
            continue;
        }
        let listing = listings.iter().find(|l| l.id == id);
        out.push(Group {
            connector: id.to_owned(),
            name: listing.map_or(id, |l| l.name.as_str()).to_owned(),
            version: listing.and_then(|l| l.version()).map(str::to_owned),
            listing,
            connections: vec![conn],
        });
    }
    out
}

/// `humanCron`: the two shapes a seeded poll uses, else the raw line.
pub fn human_cron(cron: &str) -> String {
    if cron.trim() == "* * * * *" {
        return "every minute".to_owned();
    }
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if let [m, "*", "*", "*", "*"] = fields.as_slice() {
        if let Some(n) = m.strip_prefix("*/").and_then(|n| n.parse::<u32>().ok()) {
            return if n == 1 {
                "every minute".to_owned()
            } else {
                format!("every {n} minutes")
            };
        }
    }
    cron.to_owned()
}

/// `parseLaunchSpec`: a bare package name runs through npx, else the line splits.
pub fn parse_launch_spec(spec: &str) -> (String, Vec<String>) {
    let parts: Vec<String> = spec.split_whitespace().map(str::to_owned).collect();
    let local = |t: &str| {
        t.starts_with(['.', '/', '~'])
            || t.as_bytes().get(1) == Some(&b':') && t.as_bytes()[0].is_ascii_alphabetic()
    };
    match parts.as_slice() {
        [one] if !local(one) => ("npx".to_owned(), vec!["-y".to_owned(), one.clone()]),
        [first, rest @ ..] => (first.clone(), rest.to_vec()),
        [] => (String::new(), Vec::new()),
    }
}

/// A filter chip's text: secrets masked, long values cut.
pub fn chip_value(key: &str, value: &Value, secret: bool) -> Option<String> {
    if key == "discoveredTools" {
        return None;
    }
    if secret {
        return Some("••••••".to_owned());
    }
    let text = match value {
        Value::String(t) => t.clone(),
        Value::Null => return None,
        other => other.to_string(),
    };
    Some(if text.chars().count() > 60 {
        format!("{}…", text.chars().take(57).collect::<String>())
    } else {
        text
    })
}

/// `isCredentialEnvName` or `isNeverBorrowedEnvName`: never handed over.
fn unborrowable(name: &str) -> bool {
    const SENSITIVE: [&str; 14] = [
        "AWS_SECRET",
        "AWS_SESSION",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "OPENAI_API",
        "ANTHROPIC_API",
        "GOOGLE_API",
        "STRIPE_",
        "DATABASE_URL",
        "DB_PASSWORD",
        "SECRET_",
        "PRIVATE_KEY",
        "NPM_TOKEN",
        "NODE_AUTH_TOKEN",
    ];
    let upper = name.to_uppercase();
    upper == "CLAUDECODE"
        || upper.starts_with("CLAUDE_CODE_")
        || SENSITIVE.iter().any(|p| upper.starts_with(p))
}

/// `borrowableFromManifest`: the env a signed-in CLI hands over.
pub fn borrowable(manifest: &Value) -> Vec<String> {
    let env = manifest.get("env").and_then(Value::as_array);
    strings(manifest.pointer("/auth/borrow/env"))
        .into_iter()
        .filter_map(|name| {
            env.into_iter()
                .flatten()
                .filter_map(|e| s(e, "name"))
                .find(|n| n.eq_ignore_ascii_case(&name))
                .map(str::to_owned)
        })
        .filter(|n| !unborrowable(n))
        .collect()
}

/// The activity verbs a row runs, each with the phrase it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Backfill,
    Delete,
    Run,
    Rollback,
    Remove,
    SignIn,
    SignOut,
}

impl Verb {
    pub fn phrase(self) -> &'static str {
        match self {
            Verb::Backfill => "Importing…",
            Verb::Delete | Verb::Remove => "Removing…",
            Verb::Run => "Polling…",
            Verb::Rollback => "Rolling back…",
            Verb::SignIn => "Waiting for the browser sign-in…",
            Verb::SignOut => "Signing out…",
        }
    }
}

/// What a row is doing, or the reason its last try failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    Busy(Verb),
    Failed(String),
}

/// `ActionResult`: a failure with nothing said still says something.
pub fn failure(message: Option<&str>) -> String {
    match message.map(str::trim) {
        Some(m) if !m.is_empty() => m.to_owned(),
        _ => "It did not say why.".to_owned(),
    }
}

/// A pack checked and waiting on the confirm sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub key: String,
    pub preview: Value,
}

/// A built-in connector's form (`AddConnectionForm`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuiltinForm {
    pub connector: String,
    pub values: BTreeMap<String, String>,
    pub extra: BTreeMap<String, String>,
    /// `None` while detecting; `Some(None)` when nothing was found.
    pub detected: Option<Option<(String, String)>>,
    pub owner: String,
    pub repo: String,
}

/// A packaged connector's form (`SdkConnectorForm`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SdkForm {
    /// Set when the listing named its package, so there is nothing to type.
    pub listing: Option<String>,
    pub spec: String,
    pub probing: bool,
    pub launch: Option<(String, Vec<String>)>,
    pub manifest: Option<Value>,
    pub trigger: usize,
    pub values: BTreeMap<String, String>,
    /// `None` until the CLI answers who is signed in.
    pub identity: Option<Value>,
    pub use_token: bool,
}

impl SdkForm {
    pub fn rung(&self) -> Option<Rung> {
        Rung::parse(self.manifest.as_ref()?.pointer("/auth/rung")?.as_str())
    }

    pub fn trigger(&self) -> Option<&Value> {
        self.manifest
            .as_ref()?
            .get("triggers")?
            .as_array()?
            .get(self.trigger)
    }

    /// The env the form asks for: none for a no-sign-in pack, none the borrow covers.
    pub fn fields(&self) -> Vec<&Value> {
        let Some(m) = &self.manifest else {
            return Vec::new();
        };
        let rung = self.rung();
        if rung == Some(Rung::None) {
            return Vec::new();
        }
        let covered = if rung == Some(Rung::Cli) && !self.use_token {
            borrowable(m)
        } else {
            Vec::new()
        };
        m.get("env")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|e| {
                let secret = e.get("secret") == Some(&Value::Bool(true));
                !(secret && covered.iter().any(|c| Some(c.as_str()) == s(e, "name")))
            })
            .collect()
    }

    pub fn missing(&self) -> bool {
        self.fields().iter().any(|e| {
            e.get("required") == Some(&Value::Bool(true))
                && self
                    .values
                    .get(s(e, "name").unwrap_or(""))
                    .is_none_or(|v| v.trim().is_empty())
        })
    }

    /// The `connection:create` params, with the trigger's suggested mapping.
    pub fn create_params(&self, project: Option<&str>) -> Option<Value> {
        let m = self.manifest.as_ref()?;
        let (command, args) = self.launch.as_ref()?;
        let mut plain = serde_json::Map::new();
        let mut secret = serde_json::Map::new();
        for e in self.fields() {
            let name = s(e, "name").unwrap_or("");
            let Some(v) = self
                .values
                .get(name)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            else {
                continue;
            };
            let into = if e.get("secret") == Some(&Value::Bool(true)) {
                &mut secret
            } else {
                &mut plain
            };
            into.insert(name.to_owned(), json!(v));
        }
        let trigger = self.trigger();
        let mut filters = json!({
            "command": command,
            "args": serde_json::to_string(args).ok()?,
            "env": Value::Object(plain).to_string(),
            "sdkConnectorId": s(m, "id").unwrap_or(""),
            "sdkVersion": s(m, "version").unwrap_or(""),
        });
        if let Some(icon) = m.get("icon").filter(|i| !i.is_null()) {
            filters["sdkIcon"] = json!(icon.to_string());
        }
        if let Some(t) = trigger.and_then(|t| s(t, "type")) {
            filters["sdkTrigger"] = json!(t);
        }
        if !secret.is_empty() {
            filters["secretEnv"] = json!(Value::Object(secret).to_string());
        }
        let name = s(m, "name").unwrap_or("");
        let mut params = json!({
            "connectorId": "sdk",
            "name": trigger.and_then(|t| s(t, "label")).map_or(name.to_owned(), |l| format!("{name}: {l}")),
            "filters": filters,
            "syncIntervalMinutes": 5,
            "statusMapping": status_mapping(trigger.and_then(|t| t.get("statusMapping"))),
            "executionProject": project.unwrap_or(""),
        });
        if let Some(w) = trigger
            .and_then(|t| t.get("defaultWorkflow"))
            .filter(|w| !w.is_null())
        {
            params["seedWorkflow"] = w.clone();
        }
        Some(params)
    }
}

/// `seedStatusMapping`: `[{upstream, suggestedLocal}]` as a map.
pub fn status_mapping(suggestions: Option<&Value>) -> Value {
    let mut out = serde_json::Map::new();
    for e in suggestions.and_then(Value::as_array).into_iter().flatten() {
        if let (Some(up), Some(local)) = (s(e, "upstream"), e.get("suggestedLocal")) {
            out.insert(up.to_owned(), local.clone());
        }
    }
    Value::Object(out)
}

/// The workflow editor's HTTP profile form.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileForm {
    pub name: String,
    pub base_url: String,
    pub header: String,
    pub secret: String,
    /// The connection the first save made, so a retry updates it.
    pub made: Option<String>,
    /// The last preflight: answered and what it said.
    pub result: Option<(bool, String)>,
}

impl ProfileForm {
    pub fn new(name: &str) -> Self {
        ProfileForm {
            name: name.to_owned(),
            base_url: String::new(),
            header: "Authorization: Bearer {{secret}}".to_owned(),
            secret: String::new(),
            made: None,
            result: None,
        }
    }

    pub fn ready(&self) -> bool {
        !self.name.trim().is_empty() && !self.base_url.trim().is_empty()
    }

    pub fn filters(&self) -> Value {
        let mut f = json!({
            "profileName": self.name,
            "baseUrl": self.base_url,
            "authHeader": self.header,
        });
        if !self.secret.is_empty() {
            f["secret"] = json!(self.secret);
        }
        f
    }
}

/// Which form the add box holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Form {
    Builtin(BuiltinForm),
    Sdk(SdkForm),
    Profile(ProfileForm),
    /// An MCP server listed, but no MCP connector to run it.
    NoMcp,
}

/// The add box: whose it is, what it holds, and where the save stands.
#[derive(Debug, Clone, PartialEq)]
pub struct Adding {
    /// The listing key, or the connector id for a profile.
    pub key: String,
    pub name: String,
    pub form: Form,
    /// The MCP box's "From a package" and "Manual setup" tabs.
    pub mcp_tabs: bool,
    pub project: Option<String>,
    pub saving: bool,
    pub error: Option<String>,
}

impl BuiltinForm {
    /// `canSave`.
    pub fn can_save(&self, manifest: Option<&Value>) -> bool {
        if manifest.is_some_and(|m| m.get("detectRepo") == Some(&Value::Bool(true))) {
            return matches!(self.detected, Some(Some(_)))
                || !self.owner.trim().is_empty() && !self.repo.trim().is_empty();
        }
        !manifest_fields(manifest, "auth").iter().any(|f| {
            f.get("required") == Some(&Value::Bool(true))
                && self
                    .values
                    .get(s(f, "key").unwrap_or(""))
                    .is_none_or(|v| v.trim().is_empty())
        })
    }

    /// The `connection:create` params for a built-in.
    pub fn create_params(
        &self,
        name: &str,
        manifest: Option<&Value>,
        project: Option<&str>,
    ) -> Value {
        let mut filters = serde_json::Map::new();
        let mut filter_values = BTreeMap::new();
        for f in manifest_fields(manifest, "auth") {
            let key = s(f, "key").unwrap_or("");
            if let Some(v) = self.values.get(key).filter(|v| !v.is_empty()) {
                filters.insert(key.to_owned(), json!(v));
            }
        }
        for f in manifest_fields(manifest, "taskFilters") {
            let key = s(f, "key").unwrap_or("");
            if let Some(v) = self.values.get(key).filter(|v| !v.is_empty()) {
                filters.insert(key.to_owned(), json!(v));
                filter_values.insert(key, v.as_str());
            }
        }
        for (k, v) in &self.extra {
            filters.insert(k.clone(), json!(v));
        }
        let label = if manifest.is_some_and(|m| m.get("detectRepo") == Some(&Value::Bool(true))) {
            let (owner, repo) = match &self.detected {
                Some(Some((o, r))) => (o.clone(), r.clone()),
                _ => (self.owner.trim().to_owned(), self.repo.trim().to_owned()),
            };
            filters.insert("owner".to_owned(), json!(owner));
            filters.insert("repo".to_owned(), json!(repo));
            format!("{owner}/{repo}")
        } else if let Some(p) = self
            .values
            .get("profileName")
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
        {
            p.to_owned()
        } else if let Some(team) = filter_values.get("teamKey") {
            format!("{name}: {team}")
        } else {
            match project.filter(|p| !p.is_empty()) {
                Some(p) => format!("{name} · {p}"),
                None => name.to_owned(),
            }
        };
        json!({
            "connectorId": self.connector,
            "name": label,
            "filters": filters,
            "syncIntervalMinutes": 5,
            "statusMapping": status_mapping(manifest.and_then(|m| m.get("statusMapping"))),
            "executionProject": project.unwrap_or(""),
        })
    }
}

/// A manifest's `auth` or `taskFilters` fields.
pub fn manifest_fields<'a>(manifest: Option<&'a Value>, key: &str) -> Vec<&'a Value> {
    manifest
        .and_then(|m| m.get(key))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// The page's whole state.
#[derive(Debug, Default)]
pub struct Plugins {
    pub builtins: Vec<Value>,
    pub catalog: Vec<Value>,
    pub mcp: Vec<Value>,
    pub fetched_at: Option<f64>,
    pub connections: Vec<Value>,
    pub packs: Vec<Value>,
    /// `connector:status` rows: `{connectorId, authed, message?}`.
    pub statuses: Vec<Value>,
    /// Rebuilt whenever any of the above changes.
    pub listings: Vec<Listing>,
    /// Loads still out; the first view and any pending fix wait for zero.
    pub loads: u8,
    /// Set once something answered, so the page can say it is still reading.
    pub loaded: bool,
    /// The first pack list picks the opening tab, once per visit.
    pub decided: bool,
    pub tab: Tab,
    pub selected: Option<String>,
    pub adding: Option<Adding>,
    pub query: String,
    pub category: Option<String>,
    pub kind: Option<Kind>,
    pub rung: Option<Rung>,
    pub progress: HashMap<String, Progress>,
    pub pending: Option<Pending>,
    pub installing: bool,
    /// The last install or remove report, shown under the directory.
    pub report: Option<String>,
    pub refreshing: bool,
    /// Row activity keyed by connection or pack id.
    pub activity: HashMap<String, Activity>,
    pub backfilled: HashMap<String, Result<(u64, u64), String>>,
    pub tested: HashMap<String, (bool, String)>,
    pub testing: Option<String>,
    /// Installs that finished, whose progress clears once the packs are re-read.
    pub forget: Vec<String>,
    /// A workflow fix to apply once the listings load.
    pub intent: Option<Fix>,
    /// Opened by a workflow's fix, so finishing or cancelling the form goes back to it.
    pub from_editor: bool,
}

impl Plugins {
    pub fn rebuild(&mut self) {
        self.listings = build_listings(
            &self.builtins,
            &self.catalog,
            &self.connections,
            &self.packs,
            &self.mcp,
        );
    }

    pub fn listing(&self, key: &str) -> Option<&Listing> {
        self.listings.iter().find(|l| l.key == key)
    }

    /// A listing by connector id, preferring the pack-bearing row.
    pub fn listing_by_id(&self, id: &str) -> Option<&Listing> {
        let mut found = self.listings.iter().filter(|l| l.id == id);
        let first = found.next()?;
        Some(found.find(|l| l.pack.is_some()).unwrap_or(first))
    }

    pub fn builtin(&self, id: &str) -> Option<&Value> {
        self.builtins.iter().find(|b| s(b, "id") == Some(id))
    }

    pub fn state(&self, l: &Listing) -> PackState {
        l.state(self.progress.get(&l.id))
    }

    /// The directory after the search box and pickers.
    pub fn filtered(&self) -> Vec<&Listing> {
        self.listings
            .iter()
            .filter(|l| matches_query(l, &self.query))
            .filter(|l| matches_category(l, self.category.as_deref()))
            .filter(|l| self.kind.is_none_or(|k| l.kind == k))
            .filter(|l| self.rung.is_none_or(|r| l.rung == Some(r)))
            .collect()
    }

    pub fn filtering(&self) -> bool {
        !self.query.trim().is_empty()
            || self.category.is_some()
            || self.kind.is_some()
            || self.rung.is_some()
    }

    pub fn status_of(&self, connector: &str) -> Option<&Value> {
        self.statuses
            .iter()
            .find(|st| s(st, "connectorId") == Some(connector))
    }

    pub fn busy(&self, id: &str) -> bool {
        matches!(self.activity.get(id), Some(Activity::Busy(_)))
    }

    /// The add form for a listing, as `ConnectorSettings` picks it.
    pub fn form_for(&self, l: &Listing) -> Form {
        if let Some(server) = &l.mcp {
            if self.builtin("mcp").is_none() {
                return Form::NoMcp;
            }
            let mut f = BuiltinForm {
                connector: "mcp".to_owned(),
                ..BuiltinForm::default()
            };
            f.values.insert(
                "command".into(),
                s(server, "command").unwrap_or("").to_owned(),
            );
            let args = server.get("args").cloned().unwrap_or_else(|| json!([]));
            f.values.insert("args".into(), args.to_string());
            f.extra.insert("sdkConnectorId".into(), l.id.clone());
            return Form::Builtin(f);
        }
        if l.source == Source::Builtin {
            let manifest = self.builtin(&l.id).and_then(|b| b.get("manifest"));
            let detect = manifest.is_some_and(|m| m.get("detectRepo") == Some(&Value::Bool(true)));
            if l.id == "mcp" {
                return Form::Sdk(SdkForm::default());
            }
            return Form::Builtin(BuiltinForm {
                connector: l.id.clone(),
                detected: if detect { None } else { Some(None) },
                ..BuiltinForm::default()
            });
        }
        Form::Sdk(SdkForm {
            listing: Some(l.key.clone()),
            ..SdkForm::default()
        })
    }

    /// The add box for a listing, on the first project.
    pub fn adding_for(&self, l: &Listing, project: Option<String>) -> Adding {
        let form = self.form_for(l);
        let mcp_tabs = matches!(form, Form::Builtin(_)) && l.mcp.is_some()
            || l.source == Source::Builtin && l.id == "mcp";
        let name = match (&form, l.mcp.is_some()) {
            (Form::Builtin(_), true) => self
                .builtin("mcp")
                .and_then(|b| s(b, "name"))
                .unwrap_or("MCP")
                .to_owned(),
            _ => l.name.clone(),
        };
        Adding {
            key: l.key.clone(),
            name,
            form,
            mcp_tabs,
            project,
            saving: false,
            error: None,
        }
    }

    /// Where a packaged form starts: the pack on disk, else the catalog's launch line.
    pub fn launch_for(&self, key: &str) -> Option<(String, Vec<String>)> {
        let l = self.listing(key)?;
        if let Some(path) = l.pack.as_ref().and_then(|p| s(p, "path")) {
            return Some(("node".to_owned(), vec![format!("{path}/index.js")]));
        }
        let launch = l.catalog.as_ref()?.get("launch")?;
        Some((
            s(launch, "command")?.to_owned(),
            strings(launch.get("args")),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins() -> Vec<Value> {
        vec![
            json!({"id": "http", "name": "HTTP", "capabilities": ["actions"],
                "manifest": {"auth": [{"key": "profileName", "label": "Profile name", "required": true},
                                      {"key": "baseUrl", "label": "Base URL"}]}}),
            json!({"id": "mcp", "name": "MCP", "capabilities": ["triggers", "actions"]}),
            json!({"id": "sdk", "name": "Connector package", "capabilities": [], "addable": false}),
        ]
    }

    fn catalog() -> Vec<Value> {
        vec![
            json!({"id": "ado", "name": "Azure DevOps", "packageName": "@v/ado", "version": "1.2.0",
                "category": "Work tracking", "authRung": "cli", "capabilities": ["triggers"],
                "keywords": ["boards"]}),
            json!({"id": "kusto", "name": "Azure Data Explorer", "packageName": "@v/kusto",
                "version": "0.3.0", "packUrl": "https://x/kusto.tgz", "authRung": "key",
                "verified": {"version": "0.3.0"}, "capabilities": ["actions"]}),
        ]
    }

    fn conn(id: &str, connector: &str) -> Value {
        json!({"id": id, "connectorId": connector, "name": id, "filters": {}})
    }

    #[test]
    fn listings_skip_unaddable_and_sort_ready_first() {
        let conns = [
            conn("c1", "kusto"),
            json!({"id": "c2", "connectorId": "mcp", "filters": {"implicit": true}}),
        ];
        let ls = build_listings(&builtins(), &catalog(), &conns, &[], &[]);
        let names: Vec<_> = ls.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            ["Azure Data Explorer", "MCP", "Azure DevOps", "HTTP"]
        );
        assert_eq!(ls[0].connected, 1);
        assert!(ls[1].implicit && ls[1].connected == 0);
        assert_eq!(ls[2].key, "catalog:ado");
        assert_eq!(ls[3].category, "Built in");
    }

    #[test]
    fn a_pack_overrides_the_catalog_and_a_stray_pack_gets_its_own_row() {
        let packs = [
            json!({"id": "ado", "name": "ADO", "version": "1.0.0", "auth": {"rung": "key"},
                "kind": "extension", "triggers": [], "actions": []}),
            json!({"id": "side", "name": "Side", "version": "0.1.0", "triggers": [{"type": "t", "label": "T"}],
                "actions": [], "icon": {"viewBox": "0 0 1 1", "paths": []}}),
            json!({"id": "http", "name": "Shadow", "version": "9"}),
        ];
        let ls = build_listings(&builtins(), &catalog(), &[], &packs, &[]);
        let ado = ls.iter().find(|l| l.id == "ado").unwrap();
        assert_eq!((ado.rung, ado.kind), (Some(Rung::Key), Kind::Extension));
        let side = ls.iter().find(|l| l.id == "side").unwrap();
        assert_eq!(
            (side.key.as_str(), side.category.as_str()),
            ("installed:side", "Installed")
        );
        assert_eq!(side.capabilities, ["triggers"]);
        assert_eq!(ls.iter().filter(|l| l.id == "http").count(), 1);
    }

    #[test]
    fn mcp_servers_list_as_callable() {
        let ls = build_listings(
            &[],
            &[],
            &[],
            &[],
            &[json!({"id": "fs", "name": "Files", "command": "npx"})],
        );
        assert_eq!(
            (ls[0].key.as_str(), ls[0].category.as_str()),
            ("mcp:fs", "MCP servers")
        );
        assert!(ls[0].has_actions());
    }

    #[test]
    fn pack_state_order() {
        let cat = json!({"version": "2.0.0", "packUrl": "u"});
        let pack = json!({"version": "1.0.0", "previousVersion": "0.9.0"});
        let failed = Progress {
            phase: "failed".into(),
            percent: None,
            error: None,
        };
        assert_eq!(
            pack_state(None, Some(&json!({"version": "1"})), None),
            PackState::NotReleased
        );
        assert_eq!(pack_state(None, Some(&cat), None), PackState::Absent);
        assert_eq!(pack_state(None, None, None), PackState::Absent);
        assert_eq!(
            pack_state(Some(&pack), Some(&cat), Some(&failed)),
            PackState::Rejected("The pack could not be installed".into())
        );
        let live = Progress {
            phase: "downloading".into(),
            percent: Some(40.0),
            error: None,
        };
        assert_eq!(
            describe(&pack_state(None, Some(&cat), Some(&live))).percent,
            Some(40.0)
        );
        let st = pack_state(Some(&pack), Some(&cat), Some(&Progress::phase("installed")));
        assert_eq!(
            st,
            PackState::Installed {
                version: "1.0.0".into(),
                available: Some("2.0.0".into()),
                previous: Some("0.9.0".into())
            }
        );
        let view = describe(&st);
        assert_eq!(
            (view.label.as_str(), view.tone, view.action),
            ("Update", Tone::Blocked, Some(PackAction::Update))
        );
        assert_eq!(view.detail.as_deref(), Some("v1.0.0 → 2.0.0 available"));
        // An unreleased catalog names no update.
        let st = pack_state(Some(&pack), Some(&json!({"version": "2.0.0"})), None);
        assert_eq!(describe(&st).label, "Installed");
    }

    #[test]
    fn newer_versions() {
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-beta"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(is_newer("1.0.0-beta.2", "1.0.0-beta.1"));
        assert!(is_newer("1.0.0-beta", "1.0.0-1"));
        assert!(is_newer("1.0.0-a.1", "1.0.0-a"));
        assert!(!is_newer("1.0.0+build", "1.0.0"));
    }

    #[test]
    fn can_add_follows_the_route() {
        let ls = build_listings(&builtins(), &catalog(), &[], &[], &[]);
        let by = |id: &str| ls.iter().find(|l| l.id == id).unwrap();
        assert!(by("http").can_add(&PackState::Absent));
        // Unreleased: nothing to connect to.
        let ado = by("ado");
        assert!(!ado.can_add(&ado.state(None)));
        // Released, absent, with a package name: the legacy launch.
        let kusto = by("kusto");
        assert!(kusto.can_add(&kusto.state(None)));
    }

    #[test]
    fn filters_and_facets() {
        let conns = [conn("c1", "http")];
        let ls = build_listings(&builtins(), &catalog(), &conns, &[], &[]);
        assert_eq!(
            categories(&ls),
            ["Built in", "Other", "Work tracking", CALLABLE, CONNECTED]
        );
        let hits: Vec<_> = ls
            .iter()
            .filter(|l| matches_query(l, "AZURE boards"))
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(hits, ["ado"]);
        let connected: Vec<_> = ls
            .iter()
            .filter(|l| matches_category(l, Some(CONNECTED)))
            .collect();
        assert_eq!(connected.len(), 1);
        assert_eq!(rungs(&ls), [Rung::Cli, Rung::Key]);
        assert_eq!(kinds(&ls), [Kind::Connector]);
        let secs = sections(ls.iter().collect());
        assert_eq!(secs[0].0, "Built in");
    }

    #[test]
    fn catalog_age_reads_like_a_person() {
        assert_eq!(
            catalog_age(None, 0.0),
            "Showing the connectors that shipped with Vorn"
        );
        assert_eq!(catalog_age(Some(0.0), 30_000.0), "Updated just now");
        assert_eq!(catalog_age(Some(0.0), 60_000.0), "Updated 1 minute ago");
        assert_eq!(catalog_age(Some(0.0), 7_200_000.0), "Updated 2 hours ago");
        assert_eq!(
            catalog_age(Some(0.0), 3.0 * 86_400_000.0),
            "Updated 3 days ago"
        );
    }

    #[test]
    fn crons_and_launch_specs() {
        assert_eq!(human_cron("*/5 * * * *"), "every 5 minutes");
        assert_eq!(human_cron("*/1 * * * *"), "every minute");
        assert_eq!(human_cron("* * * * *"), "every minute");
        assert_eq!(human_cron("0 9 * * 1"), "0 9 * * 1");
        assert_eq!(
            parse_launch_spec(" @v/kusto "),
            ("npx".into(), vec!["-y".into(), "@v/kusto".into()])
        );
        assert_eq!(parse_launch_spec("./run.sh"), ("./run.sh".into(), vec![]));
        assert_eq!(parse_launch_spec("C:\\x.exe"), ("C:\\x.exe".into(), vec![]));
        assert_eq!(
            parse_launch_spec("node a.js --x"),
            ("node".into(), vec!["a.js".into(), "--x".into()])
        );
    }

    #[test]
    fn chips_hide_secrets_and_cut_long_values() {
        assert_eq!(chip_value("discoveredTools", &json!("x"), false), None);
        assert_eq!(
            chip_value("k", &json!("v"), true).as_deref(),
            Some("••••••")
        );
        let long = "x".repeat(80);
        assert_eq!(
            chip_value("k", &json!(long), false).map(|v| v.chars().count()),
            Some(58)
        );
        assert_eq!(chip_value("n", &json!(3), false).as_deref(), Some("3"));
    }

    #[test]
    fn grouping_skips_implicit_and_keeps_order() {
        let conns = [
            conn("a", "kusto"),
            conn("b", "http"),
            conn("c", "kusto"),
            json!({"id": "d", "connectorId": "zz", "filters": {"implicit": "true"}}),
        ];
        let ls = build_listings(&builtins(), &catalog(), &conns, &[], &[]);
        let groups = group_connections(&conns, &ls);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            (groups[0].name.as_str(), groups[0].connections.len()),
            ("Azure Data Explorer", 2)
        );
        assert_eq!(groups[0].version.as_deref(), Some("0.3.0"));
        assert_eq!(groups[1].name, "HTTP");
    }

    #[test]
    fn extension_copy() {
        let c = json!({"panes": [{"title": "a"}, {"title": "b"}], "footers": [{}]});
        assert_eq!(describe_contributions(Some(&c)), "2 panes, 1 footer");
        assert_eq!(describe_activation(None), ["every session"]);
        let a = json!({"remoteHost": ["github.com", "x.dev"], "platform": ["darwin"]});
        assert_eq!(
            describe_activation(Some(&a)),
            ["GitHub or x.dev remotes", "macOS"]
        );
        let asked = vec!["terminal.send".to_owned(), "git.read".to_owned()];
        assert_eq!(
            permission_rows(&asked),
            [
                ("Reads", vec!["the worktree's diff and status"]),
                ("Sends", vec!["text into the session's terminal"])
            ]
        );
        assert_eq!(not_asked(&asked).len(), 4);
    }

    #[test]
    fn checked_pack_matches_only_when_everything_agrees() {
        let ls = build_listings(&[], &catalog(), &[], &[], &[]);
        let kusto = ls.iter().find(|l| l.id == "kusto").unwrap();
        let preview = json!({"id": "kusto", "version": "0.3.0", "auth": {"rung": "key"}});
        assert!(matches_listing(&preview, kusto));
        let mut other = preview.clone();
        other["installedVersion"] = json!("0.2.0");
        assert!(!matches_listing(&other, kusto));
        assert!(!matches_listing(
            &json!({"id": "kusto", "version": "0.3.0"}),
            kusto
        ));
    }

    #[test]
    fn builtin_form_names_and_filters() {
        let manifest = builtins()[0]["manifest"].clone();
        let mut f = BuiltinForm {
            connector: "http".into(),
            ..BuiltinForm::default()
        };
        assert!(!f.can_save(Some(&manifest)));
        f.values.insert("profileName".into(), " api ".into());
        f.values.insert("baseUrl".into(), "http://x".into());
        f.values.insert("stray".into(), "no".into());
        assert!(f.can_save(Some(&manifest)));
        let p = f.create_params("HTTP", Some(&manifest), Some("proj"));
        assert_eq!(p["name"], "api");
        assert_eq!(
            p["filters"],
            json!({"profileName": " api ", "baseUrl": "http://x"})
        );
        f.values.clear();
        assert_eq!(
            f.create_params("HTTP", None, Some("proj"))["name"],
            "HTTP · proj"
        );
    }

    #[test]
    fn sdk_form_splits_secrets_and_hides_borrowed_env() {
        let manifest = json!({
            "id": "ado", "name": "ADO", "version": "1.0.0",
            "auth": {"rung": "cli", "borrow": {"env": ["ado_token", "GITHUB_TOKEN"]}},
            "env": [
                {"name": "ORG", "required": true},
                {"name": "ADO_TOKEN", "secret": true, "required": true},
                {"name": "GITHUB_TOKEN", "secret": true}
            ],
            "triggers": [{"type": "wi", "label": "Work items", "statusMapping": [{"upstream": "Done", "suggestedLocal": "done"}]}]
        });
        let mut f = SdkForm {
            manifest: Some(manifest.clone()),
            launch: Some(("npx".into(), vec!["-y".into(), "@v/ado".into()])),
            ..SdkForm::default()
        };
        assert_eq!(borrowable(&manifest), ["ADO_TOKEN"]);
        let names: Vec<_> = f.fields().iter().filter_map(|e| s(e, "name")).collect();
        assert_eq!(names, ["ORG", "GITHUB_TOKEN"]);
        assert!(f.missing());
        f.values.insert("ORG".into(), "o".into());
        f.values.insert("GITHUB_TOKEN".into(), "t".into());
        assert!(!f.missing());
        let p = f.create_params(Some("proj")).unwrap();
        assert_eq!(p["name"], "ADO: Work items");
        assert_eq!(p["filters"]["env"], "{\"ORG\":\"o\"}");
        assert_eq!(p["filters"]["secretEnv"], "{\"GITHUB_TOKEN\":\"t\"}");
        assert_eq!(p["filters"]["args"], "[\"-y\",\"@v/ado\"]");
        assert_eq!(p["filters"]["sdkTrigger"], "wi");
        assert_eq!(p["statusMapping"], json!({"Done": "done"}));
        f.use_token = true;
        assert_eq!(f.fields().len(), 3);
    }

    #[test]
    fn profile_form_needs_a_name_and_url() {
        let mut p = ProfileForm::new("crm");
        assert!(!p.ready());
        p.base_url = "https://api.example.com".into();
        assert!(p.ready());
        assert!(p.filters().get("secret").is_none());
        p.secret = "s".into();
        assert_eq!(p.filters()["secret"], "s");
    }

    #[test]
    fn mcp_server_forms_prefill_the_launch_line() {
        let mut p = Plugins {
            builtins: builtins(),
            mcp: vec![json!({"id": "fs", "name": "Files", "command": "npx", "args": ["-y", "fs"]})],
            ..Plugins::default()
        };
        p.rebuild();
        let l = p.listing("mcp:fs").unwrap().clone();
        let a = p.adding_for(&l, None);
        let Form::Builtin(f) = &a.form else {
            panic!("builtin form")
        };
        assert_eq!(f.values["args"], "[\"-y\",\"fs\"]");
        assert_eq!(f.extra["sdkConnectorId"], "fs");
        assert!(a.mcp_tabs);
        assert_eq!(a.name, "MCP");
        let mcp = p.listing("mcp").unwrap().clone();
        assert!(matches!(p.adding_for(&mcp, None).form, Form::Sdk(_)));
        p.builtins.retain(|b| s(b, "id") != Some("mcp"));
        assert_eq!(p.form_for(&l), Form::NoMcp);
    }
}
