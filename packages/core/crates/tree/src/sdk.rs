//! Building trees: one function per component and chained setters for the optional parts.

use crate::{
    Badge, ButtonStyle, Choose, Entry, Kind, Meter, Node, Paint, Segment, TextStyle, Tone,
};

fn node(kind: Kind) -> Node {
    Node {
        kind,
        id: None,
        props: Default::default(),
        children: Vec::new(),
    }
}

fn holding(kind: Kind, children: impl IntoIterator<Item = Node>) -> Node {
    Node {
        children: children.into_iter().collect(),
        ..node(kind)
    }
}

/// A column of boxes.
pub fn col(children: impl IntoIterator<Item = Node>) -> Node {
    holding(Kind::Box, children)
}

/// A row of boxes, centred across.
pub fn row(children: impl IntoIterator<Item = Node>) -> Node {
    let mut n = holding(Kind::Box, children);
    n.props.row = true;
    n.props.center = true;
    n
}

pub fn text(text: impl Into<String>) -> Node {
    node(Kind::Text {
        text: text.into(),
        style: TextStyle::Body,
        tone: None,
        wrap: false,
    })
}

pub fn icon(name: impl Into<String>) -> Node {
    node(Kind::Icon {
        name: name.into(),
        tone: None,
    })
}

/// An SVG drawn in `w`×`h`, as `glyph` on cells.
pub fn shape(svg: impl Into<String>, glyph: char, w: f32, h: f32) -> Node {
    node(Kind::Shape {
        svg: svg.into(),
        glyph: glyph.to_string(),
        w,
        h,
        ink: None,
    })
}

pub fn button(id: impl Into<String>, label: impl Into<String>) -> Node {
    node(Kind::Button {
        label: label.into(),
        icon: None,
        style: ButtonStyle::Plain,
        confirm: None,
    })
    .id(id)
}

pub fn field(id: impl Into<String>, value: impl Into<String>) -> Node {
    node(Kind::Field {
        value: value.into(),
        placeholder: String::new(),
        secret: false,
    })
    .id(id)
}

/// One of `options`, each `(id, label)`; `selected` is the chosen id.
pub fn choice<I, L>(
    id: impl Into<String>,
    options: impl IntoIterator<Item = (I, L)>,
    selected: &str,
) -> Node
where
    I: Into<String>,
    L: Into<String>,
{
    let options = options
        .into_iter()
        .map(|(i, l)| {
            let id = i.into();
            Choose {
                selected: id == selected,
                id,
                label: l.into(),
            }
        })
        .collect();
    node(Kind::Choice { options }).id(id)
}

pub fn badge(text: impl Into<String>, tone: Tone) -> Node {
    node(Kind::Badge(Badge {
        text: text.into(),
        tone,
    }))
}

/// A status dot in `tone`; `.label()` names the state.
pub fn dot(tone: Tone) -> Node {
    node(Kind::Dot { tone })
}

/// A switch beside `label`, on or off.
pub fn toggle(id: impl Into<String>, label: impl Into<String>, on: bool) -> Node {
    node(Kind::Toggle {
        label: label.into(),
        on,
    })
    .id(id)
}

/// A tree drawn once per session (a badge, a card footer, a sidebar row's marks): each child for its session's id.
pub fn per_session<S: Into<String>>(children: impl IntoIterator<Item = (S, Node)>) -> Node {
    col(children.into_iter().map(|(session, n)| n.id(session)))
}

pub fn notice(text: impl Into<String>, tone: Tone) -> Node {
    node(Kind::Notice {
        text: text.into(),
        tone: Some(tone),
    })
}

pub fn stat(value: impl Into<String>, label: impl Into<String>) -> Node {
    node(Kind::Stat {
        value: value.into(),
        label: Some(label.into()),
        icon: None,
        tone: None,
        end: false,
    })
}

/// A raised card, as a page's summary of stats, a meter and its action.
pub fn card(children: impl IntoIterator<Item = Node>) -> Node {
    let mut n = col(children);
    n.props.gap = Some(12.0);
    n.props.p = Some(16.0);
    n.props.bg = Some(Paint::Token(crate::Token::Raised));
    n.props.border = Some(Paint::Token(crate::Token::Line));
    n.props.rounded = true;
    n
}

pub fn segment(value: f64, label: impl Into<String>, tone: Tone) -> Segment {
    Segment {
        value,
        label: label.into(),
        tone,
    }
}

pub fn meter(max: f64, segments: impl IntoIterator<Item = Segment>) -> Meter {
    Meter {
        max,
        segments: segments.into_iter().collect(),
        aside: None,
    }
}

impl Meter {
    /// A figure at the bar's end.
    pub fn aside(mut self, aside: impl Into<String>) -> Meter {
        self.aside = Some(aside.into());
        self
    }

    pub fn node(self) -> Node {
        node(Kind::Meter(self))
    }
}

pub fn section(title: impl Into<String>, children: impl IntoIterator<Item = Node>) -> Node {
    holding(
        Kind::Section {
            title: title.into(),
        },
        children,
    )
}

pub fn toolbar(children: impl IntoIterator<Item = Node>) -> Node {
    holding(Kind::Toolbar, children)
}

pub fn list(id: impl Into<String>, children: impl IntoIterator<Item = Node>) -> Node {
    holding(
        Kind::List {
            empty: None,
            loading: None,
            select: false,
            bulk: Vec::new(),
            lazy: false,
        },
        children,
    )
    .id(id)
}

pub fn group(heading: impl Into<String>, children: impl IntoIterator<Item = Node>) -> Node {
    holding(
        Kind::Group {
            heading: heading.into(),
            note: None,
            aside: None,
            icon: None,
            actions: Vec::new(),
        },
        children,
    )
}

pub fn entry(id: impl Into<String>, title: impl Into<String>) -> Node {
    node(Kind::Entry(Box::new(Entry {
        title: title.into(),
        selectable: true,
        ..Entry::default()
    })))
    .id(id)
}

pub fn board(columns: impl IntoIterator<Item = Node>) -> Node {
    holding(Kind::Board, columns)
}

pub fn column(title: impl Into<String>, cards: impl IntoIterator<Item = Node>) -> Node {
    holding(
        Kind::Column {
            title: title.into(),
            tone: None,
        },
        cards,
    )
}

pub fn divider() -> Node {
    node(Kind::Divider)
}

/// `items` in runs by `key`, in the order each key first shows.
pub fn group_by<T, K: PartialEq>(
    items: impl IntoIterator<Item = T>,
    key: impl Fn(&T) -> K,
) -> Vec<(K, Vec<T>)> {
    let mut out: Vec<(K, Vec<T>)> = Vec::new();
    for it in items {
        let k = key(&it);
        match out.iter_mut().find(|(g, _)| *g == k) {
            Some((_, v)) => v.push(it),
            None => out.push((k, vec![it])),
        }
    }
    out
}

/// Setters; each applies to the kinds that have it and leaves the rest alone.
impl Node {
    pub fn id(mut self, id: impl Into<String>) -> Node {
        self.id = Some(id.into());
        self
    }

    pub fn child(mut self, c: Node) -> Node {
        self.children.push(c);
        self
    }

    pub fn gap(mut self, v: f32) -> Node {
        self.props.gap = Some(v);
        self
    }

    pub fn p(mut self, v: f32) -> Node {
        self.props.p = Some(v);
        self
    }

    pub fn px(mut self, v: f32) -> Node {
        self.props.px = Some(v);
        self
    }

    pub fn py(mut self, v: f32) -> Node {
        self.props.py = Some(v);
        self
    }

    pub fn w(mut self, v: f32) -> Node {
        self.props.w = Some(v);
        self
    }

    pub fn h(mut self, v: f32) -> Node {
        self.props.h = Some(v);
        self
    }

    pub fn grow(mut self) -> Node {
        self.props.grow = true;
        self
    }

    pub fn between(mut self) -> Node {
        self.props.between = true;
        self
    }

    pub fn wrap(mut self) -> Node {
        match &mut self.kind {
            Kind::Text { wrap, .. } => *wrap = true,
            _ => self.props.wrap = true,
        }
        self
    }

    pub fn bg(mut self, p: Paint) -> Node {
        self.props.bg = Some(p);
        self
    }

    pub fn border(mut self, p: Paint) -> Node {
        self.props.border = Some(p);
        self
    }

    pub fn rounded(mut self) -> Node {
        self.props.rounded = true;
        self
    }

    pub fn label(mut self, l: impl Into<String>) -> Node {
        self.props.label = Some(l.into());
        self
    }

    pub fn style(mut self, s: TextStyle) -> Node {
        if let Kind::Text { style, .. } = &mut self.kind {
            *style = s;
        }
        self
    }

    pub fn tone(mut self, t: Tone) -> Node {
        match &mut self.kind {
            Kind::Text { tone, .. }
            | Kind::Icon { tone, .. }
            | Kind::Notice { tone, .. }
            | Kind::Stat { tone, .. }
            | Kind::Column { tone, .. } => *tone = Some(t),
            Kind::Badge(b) => b.tone = t,
            Kind::Dot { tone } => *tone = t,
            Kind::Entry(e) => e.status = Some(t),
            _ => {}
        }
        self
    }

    /// A button's or a stat's or a group's or an entry's icon.
    pub fn icon(mut self, name: impl Into<String>) -> Node {
        match &mut self.kind {
            Kind::Button { icon, .. } | Kind::Stat { icon, .. } | Kind::Group { icon, .. } => {
                *icon = Some(name.into())
            }
            Kind::Entry(e) => e.icon = Some(name.into()),
            _ => {}
        }
        self
    }

    pub fn primary(mut self) -> Node {
        if let Kind::Button { style, .. } = &mut self.kind {
            *style = ButtonStyle::Primary;
        }
        self
    }

    pub fn danger(mut self) -> Node {
        if let Kind::Button { style, .. } = &mut self.kind {
            *style = ButtonStyle::Danger;
        }
        self
    }

    /// Asks `question` before the click counts.
    pub fn confirm(mut self, question: impl Into<String>) -> Node {
        if let Kind::Button { confirm, .. } = &mut self.kind {
            *confirm = Some(question.into());
        }
        self
    }

    pub fn placeholder(mut self, p: impl Into<String>) -> Node {
        if let Kind::Field { placeholder, .. } = &mut self.kind {
            *placeholder = p.into();
        }
        self
    }

    pub fn secret(mut self) -> Node {
        if let Kind::Field { secret, .. } = &mut self.kind {
            *secret = true;
        }
        self
    }

    /// A stat at the right of its row.
    pub fn end(mut self) -> Node {
        if let Kind::Stat { end, .. } = &mut self.kind {
            *end = true;
        }
        self
    }

    /// Lets a list's entries be checked, `bulk` acting on those that are.
    pub fn select(mut self, buttons: impl IntoIterator<Item = Node>) -> Node {
        if let Kind::List { select, bulk, .. } = &mut self.kind {
            *select = true;
            *bulk = buttons.into_iter().collect();
        }
        self
    }

    pub fn empty(mut self, line: impl Into<String>) -> Node {
        if let Kind::List { empty, .. } = &mut self.kind {
            *empty = Some(line.into());
        }
        self
    }

    pub fn loading(mut self, line: impl Into<String>) -> Node {
        if let Kind::List { loading, .. } = &mut self.kind {
            *loading = Some(line.into());
        }
        self
    }

    /// Builds only the rows in view.
    pub fn lazy(mut self) -> Node {
        if let Kind::List { lazy, .. } = &mut self.kind {
            *lazy = true;
        }
        self
    }

    /// A group's quiet line beside its heading.
    pub fn note(mut self, n: impl Into<String>) -> Node {
        if let Kind::Group { note, .. } = &mut self.kind {
            *note = Some(n.into());
        }
        self
    }

    /// A group's or an entry's figure at the right.
    pub fn aside(mut self, a: impl Into<String>) -> Node {
        match &mut self.kind {
            Kind::Group { aside, .. } => *aside = Some(a.into()),
            Kind::Entry(e) => e.aside = Some(a.into()),
            _ => {}
        }
        self
    }

    /// A group's or an entry's buttons; an entry shows them on hover.
    pub fn actions(mut self, buttons: impl IntoIterator<Item = Node>) -> Node {
        match &mut self.kind {
            Kind::Group { actions, .. } => *actions = buttons.into_iter().collect(),
            Kind::Entry(e) => e.actions = buttons.into_iter().collect(),
            _ => {}
        }
        self
    }

    /// An entry's buttons behind its menu.
    pub fn more(mut self, buttons: impl IntoIterator<Item = Node>) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.more = buttons.into_iter().collect();
        }
        self
    }

    pub fn detail(mut self, d: impl Into<String>) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.detail = Some(d.into());
        }
        self
    }

    pub fn code(mut self, c: impl Into<String>) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.code = Some(c.into());
        }
        self
    }

    pub fn badge(mut self, text: impl Into<String>, tone: Tone) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.badge = Some(Badge {
                text: text.into(),
                tone,
            });
        }
        self
    }

    pub fn meter(mut self, m: Meter) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.meter = Some(m);
        }
        self
    }

    pub fn chips(mut self, chips: impl IntoIterator<Item = String>) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.chips = chips.into_iter().collect();
        }
        self
    }

    /// In a list that selects, keeps this entry from being checked.
    pub fn fixed(mut self) -> Node {
        if let Kind::Entry(e) = &mut self.kind {
            e.selectable = false;
        }
        self
    }
}
