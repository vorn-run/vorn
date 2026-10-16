//! The native backend: a frame read as a semantic tree for platform views, which act by id through [`Ui::access`].

use accesskit::Role;

use crate::backend::Backend;
use crate::element::{Content, El};
use crate::ui::Ui;
use crate::Id;

/// What a box means to a native backend.
#[derive(Debug, Clone, PartialEq)]
pub enum Sem {
    Column,
    Row,
    /// Rows a platform scrolls and separates itself.
    List,
    /// A titled group of rows.
    Section(String),
    Divider,
    /// A page shown over the screen, titled.
    Sheet(String),
    /// A menu of the buttons inside.
    More,
    /// One of a set of options, each a radio button inside.
    Choice,
    /// An icon by its name in the app's set.
    Icon(String),
    /// A dot whose tone says the state `value`.
    Status {
        value: String,
        tone: &'static str,
    },
    Badge {
        value: String,
        tone: &'static str,
    },
    /// A button with a style (`plain`, `primary`, `danger`), asking `confirm` before it runs.
    Button {
        style: &'static str,
        confirm: Option<String>,
        icon: Option<String>,
    },
    /// A row that opens the page `to`; its children are what it shows.
    Link(String),
    /// Text in a role: `title`, `body`, `secondary`, `mono`.
    Text(&'static str),
    /// A box the platform leaves to the drawn surface (a terminal).
    Drawn(String),
    /// Drawn surfaces only; a platform draws its own (a section's title, a toggle's label).
    Hidden,
}

/// One node of the semantic tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    /// What the box is, e.g. `list`, `section`, `toggle`, `field`, `link`, `text` or `drawn`.
    pub kind: &'static str,
    pub id: Option<Id>,
    pub label: String,
    pub value: String,
    pub on: Option<bool>,
    pub secret: bool,
    pub style: &'static str,
    pub confirm: Option<String>,
    pub children: Vec<Node>,
}

impl Node {
    fn of(kind: &'static str) -> Node {
        Node {
            kind,
            ..Node::default()
        }
    }

    /// Every node under this one and itself, depth first.
    pub fn walk(&self) -> Vec<&Node> {
        let mut out = vec![self];
        for c in &self.children {
            out.extend(c.walk());
        }
        out
    }

    /// The tree as JSON, ids as strings (they do not fit a double).
    pub fn to_json(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    fn write(&self, s: &mut String) {
        s.push_str("{\"k\":");
        quote(s, self.kind);
        if let Some(id) = self.id {
            s.push_str(&format!(",\"id\":\"{}\"", id.0));
        }
        for (k, v) in [
            ("label", self.label.as_str()),
            ("value", self.value.as_str()),
            ("style", self.style),
        ] {
            if !v.is_empty() {
                s.push_str(&format!(",\"{k}\":"));
                quote(s, v);
            }
        }
        if let Some(on) = self.on {
            s.push_str(&format!(",\"on\":{on}"));
        }
        if self.secret {
            s.push_str(",\"secret\":true");
        }
        if let Some(c) = &self.confirm {
            s.push_str(",\"confirm\":");
            quote(s, c);
        }
        if !self.children.is_empty() {
            s.push_str(",\"c\":[");
            for (i, c) in self.children.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                c.write(s);
            }
            s.push(']');
        }
        s.push('}');
    }
}

fn quote(s: &mut String, v: &str) {
    s.push('"');
    for ch in v.chars() {
        match ch {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
}

impl Ui {
    /// A UI whose frames are laid out on the CPU and read as a semantic tree for [`Native`].
    pub fn native(cfg: &crate::UiConfig) -> Ui {
        let mut ui = Ui::headless(crate::RenderMode::Cpu, cfg);
        ui.native = Some(Node::of("column"));
        ui
    }

    /// Whether frames are read as a semantic tree, with menus and fields shown whether open or not.
    pub fn is_native(&self) -> bool {
        self.native.is_some()
    }

    /// The last frame's semantic tree, on a native UI.
    pub fn native_tree(&self) -> Option<&Node> {
        self.native.as_ref()
    }

    pub(crate) fn record_native(&mut self, root: &El) {
        if self.native.is_some() {
            let mut kids = Vec::new();
            self.sem(root, &mut kids);
            let root = match <[Node; 1]>::try_from(kids) {
                Ok([one]) if one.kind == "column" => one,
                Ok([one]) => Node {
                    children: vec![one],
                    ..Node::of("column")
                },
                Err(kids) => Node {
                    children: kids,
                    ..Node::of("column")
                },
            };
            self.native = Some(root);
        }
    }

    /// `el` as nodes pushed to `out`: one, or its children's when it means nothing itself.
    fn sem(&self, el: &El, out: &mut Vec<Node>) {
        let kids = |ui: &Ui| {
            let mut v = Vec::new();
            for c in &el.children {
                ui.sem(c, &mut v);
            }
            v
        };
        let label = el.a11y.label.clone().unwrap_or_default();
        if let Some(sem) = el.sem.as_deref() {
            let mut n = match sem {
                Sem::Hidden => return,
                Sem::Column => Node::of("column"),
                Sem::Row => Node::of("row"),
                Sem::List => Node::of("list"),
                Sem::Divider => Node::of("divider"),
                Sem::More => Node::of("more"),
                Sem::Choice => Node::of("choice"),
                Sem::Section(t) => Node {
                    label: t.clone(),
                    ..Node::of("section")
                },
                Sem::Sheet(t) => Node {
                    label: t.clone(),
                    ..Node::of("sheet")
                },
                Sem::Link(to) => Node {
                    value: to.clone(),
                    ..Node::of("link")
                },
                Sem::Drawn(what) => Node {
                    value: what.clone(),
                    ..Node::of("drawn")
                },
                Sem::Icon(name) => {
                    out.push(Node {
                        value: name.clone(),
                        ..Node::of("icon")
                    });
                    return;
                }
                Sem::Status { value, tone } | Sem::Badge { value, tone } => {
                    let kind = if matches!(sem, Sem::Status { .. }) {
                        "status"
                    } else {
                        "badge"
                    };
                    out.push(Node {
                        value: value.clone(),
                        style: tone,
                        ..Node::of(kind)
                    });
                    return;
                }
                Sem::Text(role) => {
                    out.push(Node {
                        label: text_of(el).unwrap_or(label),
                        style: role,
                        ..Node::of("text")
                    });
                    return;
                }
                Sem::Button {
                    style,
                    confirm,
                    icon,
                } => {
                    out.push(Node {
                        id: el.id,
                        label,
                        value: icon.clone().unwrap_or_default(),
                        style,
                        confirm: confirm.clone(),
                        ..Node::of("button")
                    });
                    return;
                }
            };
            n.id = el.id;
            if n.kind != "drawn" {
                n.children = kids(self);
            }
            out.push(n);
            return;
        }
        let control = |kind| Node {
            id: el.id,
            label: label.clone(),
            on: el.a11y.toggled,
            ..Node::of(kind)
        };
        match (el.a11y.role, &el.content) {
            (_, Content::Edit(look)) => {
                let value = el.id.map(|id| self.edit_text(id).to_owned());
                out.push(Node {
                    id: el.id,
                    label: look.placeholder.clone(),
                    value: if look.secure {
                        String::new()
                    } else {
                        value.unwrap_or_default()
                    },
                    secret: look.secure,
                    ..Node::of("field")
                });
            }
            (Some(Role::Switch), _) => out.push(control("toggle")),
            (Some(Role::RadioButton), _) => out.push(control("option")),
            (Some(Role::Button | Role::MenuItem | Role::Link), _) if el.id.is_some() => {
                out.push(control("button"));
            }
            (_, Content::Text { text, .. }) => out.push(Node {
                label: text.clone(),
                style: "body",
                ..Node::of("text")
            }),
            _ => out.extend(kids(self)),
        }
    }
}

/// The text a box shows, its own or its first text child's.
fn text_of(el: &El) -> Option<String> {
    if let Content::Text { text, .. } = &el.content {
        return Some(text.clone());
    }
    el.children.iter().find_map(text_of)
}

/// The last frame's semantic tree as JSON, for a platform to draw with its own views.
#[derive(Default)]
pub struct Native {
    pub json: String,
}

impl Backend for Native {
    fn present(&mut self, ui: &mut Ui) -> Result<bool, String> {
        let Some(tree) = ui.native_tree() else {
            return Err("a native backend needs a native Ui (Ui::native)".into());
        };
        let json = tree.to_json();
        let changed = json != self.json;
        self.json = json;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{div, text, widgets, Action, UiConfig};

    #[test]
    fn reads_controls_from_roles_and_structure_from_tags() {
        let mut ui = Ui::native(&UiConfig::system(1.0, 13.0));
        let field = Id::new("f");
        let root = div()
            .col()
            .child(
                div()
                    .sem(Sem::Section("Account \"main\"".into()))
                    .child(text("Account", 13.0, crate::theme::color::WHITE).sem(Sem::Hidden))
                    .child(widgets::toggle(Id::new("t"), "Count", true, false))
                    .child(widgets::text_input(&ui, field, "Key", "Key").secure())
                    .child(
                        div()
                            .id(Id::new("b"))
                            .on(Action::Click)
                            .role(Role::Button, "Go"),
                    ),
            )
            .child(div().child(text("loose", 13.0, crate::theme::color::WHITE)));
        ui.begin(crate::Rgba::TRANSPARENT);
        ui.layout(root, (390.0, 844.0));
        let mut b = Native::default();
        assert!(b.present(&mut ui).unwrap());
        let tree = ui.native_tree().unwrap();
        let kinds: Vec<_> = tree.walk().iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds,
            ["column", "section", "toggle", "field", "button", "text"]
        );
        let section = &tree.children[0];
        assert_eq!(section.label, "Account \"main\"");
        assert_eq!(section.children[0].on, Some(true));
        assert!(section.children[1].secret);
        assert_eq!(section.children[2].id, Some(Id::new("b")));
        assert!(
            b.json.contains(r#""label":"Account \"main\"""#),
            "{}",
            b.json
        );
        assert!(
            !b.present(&mut ui).unwrap(),
            "the same frame twice is no change"
        );
    }
}
