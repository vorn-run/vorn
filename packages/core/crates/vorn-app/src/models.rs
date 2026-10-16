//! The launcher's model picker (`ModelPicker`): what vornd's
//! `agent:listModels` answer holds, the rows the menu lists for a query, its
//! footer line, and the check a typed model id must pass.

use serde_json::Value;

/// Shown under the id field when a typed id cannot be passed to the CLI.
pub const INVALID_ID: &str =
    "Enter a model id without spaces, control characters, or a leading dash.";

/// A model id goes on a command line as one argument, so it must read as
/// one (`validateModelId`).
pub fn validate_id(value: &str) -> Result<&str, &'static str> {
    let id = value.trim();
    let bad = |c: char| c.is_whitespace() || c < ' ' || c == '\u{7f}';
    if id.is_empty() || id.starts_with('-') || id.chars().any(bad) {
        return Err(INVALID_ID);
    }
    Ok(id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    /// The last list, kept after asking again failed.
    Stale,
    Unavailable,
}

/// One `agent:listModels` answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub choices: Vec<Choice>,
    pub status: Status,
    /// Milliseconds since the epoch.
    pub fetched_at: Option<u64>,
    pub error: Option<String>,
}

impl Catalog {
    /// What a failed call reads as, as the renderer's hook words it.
    pub fn failed() -> Self {
        Self::unavailable("Could not list models.")
    }

    fn unavailable(error: &str) -> Self {
        Self {
            choices: Vec::new(),
            status: Status::Unavailable,
            fetched_at: None,
            error: Some(error.to_owned()),
        }
    }

    pub fn from_json(v: &Value) -> Self {
        let status = match v.get("status").and_then(Value::as_str) {
            Some("ready") => Status::Ready,
            Some("stale") => Status::Stale,
            _ => Status::Unavailable,
        };
        let text = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
        let choices = v
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let id = text(c, "id")?;
                Some(Choice {
                    label: text(c, "label").unwrap_or_else(|| id.clone()),
                    description: text(c, "description"),
                    id,
                })
            })
            .collect();
        Self {
            choices,
            status,
            fetched_at: v.get("fetchedAt").and_then(Value::as_f64).map(|f| f as u64),
            error: text(v, "error"),
        }
    }
}

/// What the launcher knows of one agent's models in one project.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub catalog: Option<Catalog>,
    /// Asked and not yet answered: the first time, or a refresh.
    pub loading: bool,
}

impl Listing {
    /// The label the trigger shows for `value`: the listed label, else the id.
    pub fn label<'a>(&'a self, value: &'a str) -> &'a str {
        self.choices()
            .iter()
            .find(|c| c.id == value)
            .map_or(value, |c| c.label.as_str())
    }

    fn choices(&self) -> &[Choice] {
        self.catalog.as_ref().map_or(&[], |c| &c.choices)
    }

    /// The choices a query keeps: those whose label or id holds it.
    pub fn shown(&self, query: &str) -> Vec<&Choice> {
        let needle = query.trim().to_lowercase();
        self.choices()
            .iter()
            .filter(|c| {
                needle.is_empty()
                    || format!("{} {}", c.label, c.id)
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect()
    }

    /// The listed choice whose id is the query, which Enter picks.
    pub fn exact(&self, query: &str) -> Option<&Choice> {
        let needle = query.trim().to_lowercase();
        self.shown(query)
            .into_iter()
            .find(|c| c.id.to_lowercase() == needle)
    }
}

/// One line of the menu's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row<'a> {
    /// "Asking {agent} for its models…", before the first answer.
    Asking,
    /// "Agent default", picked when no model is set.
    Default,
    /// A provider's header, for an agent whose ids are `provider/model`.
    Provider(&'a str),
    Model {
        id: &'a str,
        label: &'a str,
        description: Option<&'a str>,
    },
    /// "No listed model matches".
    NoMatch,
}

/// A `provider/model` id's provider, which becomes a header.
fn provider_of(id: &str) -> Option<&str> {
    id.find('/').filter(|&i| i > 0).map(|i| &id[..i])
}

/// The menu's rows for the model set (`value`) and what is typed; `group`
/// heads each provider's models when the agent's ids are `provider/model`.
pub fn rows<'a>(
    group: bool,
    listing: &'a Listing,
    value: Option<&'a str>,
    query: &str,
) -> Vec<Row<'a>> {
    let typed = !query.trim().is_empty();
    let shown = listing.shown(query);
    let mut out = Vec::new();
    if listing.loading && listing.catalog.is_none() {
        out.push(Row::Asking);
    }
    if !typed {
        out.push(Row::Default);
        if let Some(v) = value.filter(|v| !listing.choices().iter().any(|c| c.id == *v)) {
            out.push(Row::Model {
                id: v,
                label: v,
                description: None,
            });
        }
    }
    let mut last = None;
    for c in &shown {
        let provider = group.then(|| provider_of(&c.id)).flatten();
        if provider.is_some() && provider != last {
            out.push(Row::Provider(provider.unwrap_or_default()));
            last = provider;
        }
        out.push(Row::Model {
            id: &c.id,
            label: &c.label,
            description: c.description.as_deref(),
        });
    }
    if typed && shown.is_empty() {
        out.push(Row::NoMatch);
    }
    out
}

/// The footer's line and whether it spins, as of `opened_at` (ms).
pub fn footer(agent: &str, listing: &Listing, opened_at: u64) -> (String, bool) {
    let Some(catalog) = &listing.catalog else {
        return (String::new(), false);
    };
    if listing.loading {
        return (format!("Asking {agent}\u{2026}"), true);
    }
    if catalog.status == Status::Unavailable {
        let text = catalog
            .error
            .clone()
            .unwrap_or_else(|| "Could not list models \u{2014} type an id".to_owned());
        return (text, false);
    }
    let ago = match catalog.fetched_at {
        None | Some(0) => String::new(),
        Some(at) => match (opened_at.saturating_sub(at) as f64 / 60_000.0).round() as u64 {
            0 => "just now".to_owned(),
            m => format!("{m} min ago"),
        },
    };
    let failed = if catalog.status == Status::Stale {
        " \u{b7} refreshing failed"
    } else {
        ""
    };
    (format!("From {agent} \u{b7} {ago}{failed}"), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listing(v: Value) -> Listing {
        Listing {
            catalog: Some(Catalog::from_json(&v)),
            loading: false,
        }
    }

    #[test]
    fn ids_must_read_as_one_argument() {
        assert_eq!(validate_id("  sonnet  "), Ok("sonnet"));
        assert_eq!(validate_id("openai/gpt-5"), Ok("openai/gpt-5"));
        for bad in [
            "",
            "   ",
            "-p",
            "two words",
            "tab\there",
            "bell\u{7}",
            "del\u{7f}",
        ] {
            assert_eq!(validate_id(bad), Err(INVALID_ID), "{bad:?}");
        }
    }

    #[test]
    fn query_filters_on_label_and_id_and_finds_the_exact_id() {
        let l = listing(json!({
            "status": "ready", "fetchedAt": 1,
            "choices": [{"id": "sonnet", "label": "Sonnet 4.5"}, {"id": "opus", "label": "Opus"}]
        }));
        let ids = |q: &str| l.shown(q).iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(""), ["sonnet", "opus"]);
        assert_eq!(ids("4.5"), ["sonnet"]);
        assert_eq!(ids(" OPUS "), ["opus"]);
        assert_eq!(l.exact("Opus").map(|c| c.id.as_str()), Some("opus"));
        assert_eq!(l.exact("son"), None);
        assert_eq!(l.label("sonnet"), "Sonnet 4.5");
        assert_eq!(l.label("custom"), "custom");
    }

    #[test]
    fn rows_put_default_and_an_unlisted_value_first_and_group_providers() {
        let l = listing(json!({
            "status": "ready",
            "choices": [{"id": "openai/gpt-5", "label": "GPT-5"}, {"id": "openai/o3", "label": "o3"},
                        {"id": "anthropic/sonnet", "label": "Sonnet"}]
        }));
        let rows = rows(true, &l, Some("mine"), "");
        assert_eq!(rows[0], Row::Default);
        assert!(matches!(rows[1], Row::Model { id: "mine", .. }));
        assert_eq!(rows[2], Row::Provider("openai"));
        assert_eq!(rows[5], Row::Provider("anthropic"));
        assert_eq!(rows.len(), 7);
        assert_eq!(super::rows(true, &l, None, "zzz"), [Row::NoMatch]);
        assert!(!super::rows(false, &l, None, "").contains(&Row::Provider("openai")));
        let asking = Listing {
            catalog: None,
            loading: true,
        };
        assert_eq!(
            super::rows(false, &asking, None, ""),
            [Row::Asking, Row::Default]
        );
    }

    #[test]
    fn footer_says_where_the_list_came_from() {
        let at = 10 * 60_000;
        let ready = listing(json!({"status": "ready", "choices": [], "fetchedAt": at}));
        assert_eq!(
            footer("acme", &ready, at + 20_000).0,
            "From acme \u{b7} just now"
        );
        assert_eq!(
            footer("acme", &ready, at + 3 * 60_000).0,
            "From acme \u{b7} 3 min ago"
        );
        let stale = listing(json!({"status": "stale", "choices": [], "fetchedAt": at}));
        assert!(footer("beta", &stale, at)
            .0
            .ends_with("just now \u{b7} refreshing failed"));
        let gone = listing(
            json!({"status": "unavailable", "choices": [], "error": "beta is not installed"}),
        );
        assert_eq!(footer("beta", &gone, 0).0, "beta is not installed");
        let bare = listing(json!({"status": "unavailable", "choices": []}));
        assert_eq!(
            footer("beta", &bare, 0).0,
            "Could not list models \u{2014} type an id"
        );
        let refreshing = Listing {
            loading: true,
            ..ready
        };
        assert_eq!(
            footer("acme", &refreshing, 0),
            ("Asking acme\u{2026}".to_owned(), true)
        );
        assert_eq!(
            footer("acme", &Listing::default(), 0),
            (String::new(), false)
        );
    }
}
