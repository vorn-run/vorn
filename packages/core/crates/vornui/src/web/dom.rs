//! The accessibility mirror: it turns vornui's AccessKit tree into DOM nodes with ARIA roles,
//! since AccessKit has no adapter for the web.

use std::collections::{HashMap, HashSet};

use crate::ROOT_NODE;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId, TreeUpdate};
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement};

fn aria(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Button => "button",
        Role::CheckBox => "checkbox",
        Role::Image => "img",
        Role::Link => "link",
        Role::Terminal | Role::TextInput | Role::MultilineTextInput => "textbox",
        Role::Menu => "menu",
        Role::MenuItem => "menuitem",
        Role::Tab => "tab",
        Role::TabList => "tablist",
        Role::ComboBox => "combobox",
        Role::Group => "group",
        Role::Tooltip => "tooltip",
        _ => return None,
    })
}

/// A mirrored node and the state it last showed, so a frame that changed
/// nothing touches no DOM.
struct Mirrored {
    el: HtmlElement,
    shown: String,
}

pub struct A11yMirror {
    doc: Document,
    root: HtmlElement,
    nodes: HashMap<NodeId, Mirrored>,
}

impl A11yMirror {
    pub fn new(doc: Document, root: HtmlElement) -> A11yMirror {
        A11yMirror {
            doc,
            root,
            nodes: HashMap::new(),
        }
    }

    /// Brings the DOM in line with `tree`, a whole tree as vornui builds
    /// one each frame, laid out at device scale `k`.
    pub fn update(&mut self, tree: &TreeUpdate, k: f32) {
        let by_id: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
        let k = f64::from(k);
        let mut seen = HashSet::new();
        let mut stack = vec![(ROOT_NODE, None::<NodeId>, (0.0f64, 0.0f64))];
        while let Some((id, parent, origin)) = stack.pop() {
            let Some(node) = by_id.get(&id) else {
                continue;
            };
            seen.insert(id);
            let b = node.bounds().unwrap_or_default();
            let (x, y) = (b.x0 / k, b.y0 / k);
            let rect = (x - origin.0, y - origin.1, b.width() / k, b.height() / k);
            let host = if id == ROOT_NODE {
                None
            } else {
                self.put(id, parent, node, rect, tree.focus == id);
                Some(id)
            };
            let o = if host.is_some() { (x, y) } else { origin };
            for c in node.children().iter().rev() {
                stack.push((*c, host, o));
            }
        }
        self.nodes.retain(|id, m| {
            let keep = seen.contains(id);
            if !keep {
                m.el.remove();
            }
            keep
        });
    }

    fn put(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        n: &Node,
        r: (f64, f64, f64, f64),
        focus: bool,
    ) {
        let role = aria(n.role());
        let label = n.label().unwrap_or("");
        let value = n.value().unwrap_or("");
        let shown = format!(
            "{role:?}|{label}|{value}|{:.0},{:.0},{:.0},{:.0}|{parent:?}|{focus}",
            r.0, r.1, r.2, r.3
        );
        if self.nodes.get(&id).is_some_and(|m| m.shown == shown) {
            return;
        }
        let Some(el) = self.element(id, parent) else {
            return;
        };
        let style = el.style();
        let _ = style.set_property("left", &format!("{:.1}px", r.0));
        let _ = style.set_property("top", &format!("{:.1}px", r.1));
        let _ = style.set_property("width", &format!("{:.1}px", r.2));
        let _ = style.set_property("height", &format!("{:.1}px", r.3));
        set_attr(&el, "role", role);
        if n.role() == Role::Terminal {
            let _ = el.set_attribute("aria-readonly", "true");
            let _ = el.set_attribute("aria-multiline", "true");
            let _ = el.set_attribute("aria-roledescription", "terminal");
        }
        set_attr(&el, "aria-label", (!label.is_empty()).then_some(label));
        // A node with no ARIA role is static text: its label is what it says.
        let text = if role.is_none() { label } else { value };
        set_own_text(&el, text);
        set_attr(&el, "data-focused", focus.then_some(""));
        if let Some(m) = self.nodes.get_mut(&id) {
            m.shown = shown;
        }
    }

    /// The node's element, made under its parent's on first use and moved
    /// there if the parent changed.
    fn element(&mut self, id: NodeId, parent: Option<NodeId>) -> Option<HtmlElement> {
        let host: Element = parent
            .and_then(|p| self.nodes.get(&p))
            .map_or_else(|| self.root.clone().into(), |m| m.el.clone().into());
        if let Some(m) = self.nodes.get(&id) {
            if m.el.parent_element().as_ref() != Some(&host) {
                let _ = host.append_child(&m.el);
            }
            return Some(m.el.clone());
        }
        let el: HtmlElement = self.doc.create_element("div").ok()?.dyn_into().ok()?;
        let _ = el.set_attribute("data-node", &id.0.to_string());
        let _ = host.append_child(&el);
        self.nodes.insert(
            id,
            Mirrored {
                el: el.clone(),
                shown: String::new(),
            },
        );
        Some(el)
    }
}

fn set_attr(el: &HtmlElement, name: &str, value: Option<&str>) {
    let _ = match value {
        Some(v) => el.set_attribute(name, v),
        None => el.remove_attribute(name),
    };
}

/// Sets the element's own text node, leaving its child elements alone.
fn set_own_text(el: &HtmlElement, text: &str) {
    let first = el.first_child();
    match first {
        Some(t) if t.node_type() == web_sys::Node::TEXT_NODE => {
            if t.node_value().as_deref() != Some(text) {
                t.set_node_value(Some(text));
            }
        }
        _ if text.is_empty() => {}
        _ => {
            if let Some(doc) = el.owner_document() {
                let t = doc.create_text_node(text);
                let _ = el.insert_before(&t, first.as_ref());
            }
        }
    }
}

/// What an assistive technology's click on mirrored node `id` asks.
pub fn click(id: u64) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: NodeId(id),
        data: None,
    }
}
