//! The agents a session can start, as vornd's `agent:list` describes them
//! from their adapters: names, marks, install commands, what each can do,
//! and how each connects to Vorn's tools. The app knows no agent by name;
//! with every adapter off the list is empty and the launcher opens a shell.

use serde_json::Value;

/// How an agent gets Vorn's skills and MCP server (`AgentMcpSetup`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpSetup {
    /// In order: some agents want a marketplace added first.
    pub commands: Vec<String>,
    /// Typed inside the agent, not in a shell.
    pub in_agent: bool,
    pub note: Option<String>,
}

/// One agent, as the launcher, the pickers and Settings draw it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentInfo {
    /// Its `AiAgentType`.
    pub key: String,
    /// What Settings and the launcher call it.
    pub name: String,
    /// What menus, the palette and the task board call it.
    pub label: String,
    /// What a workflow step calls it.
    pub short_name: String,
    /// The card's accent, and the tint behind its mark.
    pub color: u32,
    pub bg_alpha: f32,
    pub description: String,
    pub install: String,
    pub url: String,
    pub command: String,
    pub args: Vec<String>,
    pub headless: Vec<String>,
    /// One character for its mark where only text is drawn.
    pub mark: Option<String>,
    /// Its mark as SVG text.
    pub icon: Option<String>,
    /// A model can be chosen for it.
    pub models: bool,
    /// Its model ids are `provider/model`, grouped by provider.
    pub group_models: bool,
    /// It can run headless.
    pub runs_headless: bool,
    /// A past session can be resumed by its id.
    pub resumes: bool,
    pub mcp: Option<McpSetup>,
    /// Variables it sets in the sessions it runs, never handed on.
    pub markers: (Vec<String>, Vec<String>),
    /// The extension it comes in, and whether Vorn ships it.
    pub extension: String,
    pub builtin: bool,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// `#rrggbb` as a number, `#rrggbbaa`'s alpha as a fraction.
fn hex(v: Option<&str>) -> Option<(u32, Option<f32>)> {
    let digits = v?.strip_prefix('#')?;
    let n = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some((n, None)),
        8 => Some((n >> 8, Some((n & 0xff) as f32 / 255.0))),
        _ => None,
    }
}

impl AgentInfo {
    /// One entry of `agent:list`; `None` without an id.
    pub fn from_json(v: &Value) -> Option<AgentInfo> {
        let key = text(v, "id");
        if key.is_empty() {
            return None;
        }
        let name = Some(text(v, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| key.clone());
        let label = Some(text(v, "label"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| name.clone());
        let short_name = Some(text(v, "shortName"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| label.clone());
        let (color, _) = hex(v.get("color").and_then(Value::as_str)).unwrap_or((0xffffff, None));
        let bg_alpha = hex(v.get("tint").and_then(Value::as_str))
            .and_then(|(_, a)| a)
            .unwrap_or(0x20 as f32 / 255.0);
        let mcp = v.get("mcp").filter(|m| m.is_object()).map(|m| McpSetup {
            commands: strings(m.get("commands")),
            in_agent: m.get("inAgent").and_then(Value::as_bool).unwrap_or(false),
            note: m.get("note").and_then(Value::as_str).map(str::to_owned),
        });
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(AgentInfo {
            name,
            label,
            short_name,
            color,
            bg_alpha,
            description: text(v, "description"),
            install: text(v, "install"),
            url: text(v, "url"),
            command: Some(text(v, "command"))
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| key.clone()),
            args: strings(v.get("args")),
            headless: strings(v.get("headlessArgs")),
            mark: v.get("mark").and_then(Value::as_str).map(str::to_owned),
            icon: v.get("icon").and_then(Value::as_str).map(str::to_owned),
            models: flag("models"),
            group_models: flag("groupModels"),
            runs_headless: flag("headless"),
            resumes: flag("resumes"),
            mcp,
            markers: (
                strings(v.pointer("/envMarkers/names")),
                strings(v.pointer("/envMarkers/prefixes")),
            ),
            extension: text(v, "extension"),
            builtin: flag("builtin"),
            key,
        })
    }
}

/// The agents that are on, in the launcher's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Agents {
    list: Vec<AgentInfo>,
    /// vornd answered `agent:list` at least once.
    pub known: bool,
}

impl Agents {
    /// `agent:list`'s answer.
    pub fn from_json(v: &Value) -> Agents {
        Agents {
            list: v
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(AgentInfo::from_json)
                .collect(),
            known: true,
        }
    }

    pub fn new(list: Vec<AgentInfo>) -> Agents {
        Agents { list, known: true }
    }

    pub fn get(&self, key: &str) -> Option<&AgentInfo> {
        self.list.iter().find(|a| a.key == key)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, AgentInfo> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// What menus call `key`: its label, or the key for one that is off.
    pub fn label<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).map_or(key, |a| a.label.as_str())
    }

    /// What the launcher calls `key`: its name.
    pub fn name<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).map_or(key, |a| a.name.as_str())
    }

    /// What a workflow step calls `key`.
    pub fn short_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).map_or(key, |a| a.short_name.as_str())
    }

    /// Whether a past session of `key` can be resumed by its id
    /// (`supportsExactSessionResume`).
    pub fn resumes(&self, key: &str) -> bool {
        self.get(key).is_some_and(|a| a.resumes)
    }

    /// Whether a model can be chosen for `key`.
    pub fn selects_models(&self, key: &str) -> bool {
        self.get(key).is_some_and(|a| a.models)
    }

    /// The agent a new session starts when none is chosen: the first, or a
    /// shell with every adapter off.
    pub fn default_key(&self) -> &str {
        self.list.first().map_or("shell", |a| a.key.as_str())
    }

    /// Whether `name` is a variable an agent sets in its sessions.
    pub fn is_marker(&self, name: &str) -> bool {
        let upper = name.to_uppercase();
        self.list.iter().any(|a| {
            a.markers.0.iter().any(|m| m.to_uppercase() == upper)
                || a.markers
                    .1
                    .iter()
                    .any(|p| upper.starts_with(&p.to_uppercase()))
        })
    }
}

impl<'a> IntoIterator for &'a Agents {
    type Item = &'a AgentInfo;
    type IntoIter = std::slice::Iter<'a, AgentInfo>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

#[doc(hidden)]
pub mod testing {
    //! Agents for this crate's tests, as `agent:list` answers them, named
    //! for no agent CLI.

    use serde_json::json;

    use super::*;

    /// `acme` lists models and resumes; `beta` groups models by provider;
    /// `gamma` has no models.
    pub fn agents() -> Agents {
        Agents::from_json(&json!([
            { "id": "acme", "name": "Acme Code", "label": "Acme Code", "shortName": "Acme",
              "description": "Acme CLI", "install": "npm install -g acme", "url": "https://acme.test",
              "command": "acme", "args": [], "headlessArgs": ["--yes"], "color": "#d97757",
              "tint": "#d9775720", "mark": "*", "models": true, "headless": true, "resumes": true,
              "mcp": { "commands": ["/plugin install vorn"], "inAgent": true },
              "envMarkers": { "names": ["ACMECODE"], "prefixes": ["ACME_CODE_"] },
              "extension": "test.acme", "builtin": true },
            { "id": "beta", "name": "Beta CLI", "label": "Beta", "shortName": "Beta",
              "command": "beta", "color": "#7a9dff", "models": true, "groupModels": true,
              "headless": true, "resumes": true, "mcp": { "commands": ["beta mcp add vorn"], "note": "Restart it." },
              "extension": "test.beta", "builtin": true },
            { "id": "gamma", "name": "Gamma CLI", "label": "Gamma", "command": "gamma",
              "color": "#3186ff", "headless": true, "extension": "test.gamma", "builtin": true }
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_list_vornd_answers() {
        let agents = testing::agents();
        assert_eq!(agents.len(), 3);
        let acme = agents.get("acme").unwrap();
        assert_eq!((acme.color, acme.bg_alpha), (0xd97757, 0x20 as f32 / 255.0));
        assert_eq!(acme.headless, ["--yes"]);
        assert!(acme.mcp.as_ref().is_some_and(|m| m.in_agent));
        assert_eq!(agents.label("beta"), "Beta");
        assert_eq!(agents.name("gone"), "gone");
        assert_eq!(agents.short_name("gamma"), "Gamma");
        assert!(agents.selects_models("acme") && !agents.selects_models("gamma"));
        assert_eq!(agents.default_key(), "acme");
        assert_eq!(Agents::default().default_key(), "shell");
        assert!(agents.is_marker("acme_code_port") && agents.is_marker("AcmeCode"));
        assert!(!agents.is_marker("PATH"));
    }
}
