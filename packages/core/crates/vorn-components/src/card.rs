//! A session card's chrome (`CardActionCluster`): the header's icon
//! buttons, with the close asking first (`ConfirmPopover`).

use vornui::accesskit::Role;
use vornui::theme::{color, radius};
use vornui::widgets::{confirm, tooltip_at, Tone};
use vornui::{div, Action, El, Id, Place, Rgba, Ui};

/// Where the cluster sits: on a grid card, or on the focused stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardVariant {
    /// A grid card: it can be minimized, expands, and its tooltips sit
    /// above the buttons.
    Mini,
    /// The focused stage: it collapses back to the grid, tooltips below.
    Focused,
}

impl CardVariant {
    /// The side the cluster's tooltips sit on.
    pub fn tip_place(self) -> Place {
        match self {
            CardVariant::Mini => Place::Above,
            CardVariant::Focused => Place::Below,
        }
    }
}

/// One of the cluster's buttons: `p-1 rounded hover:bg-white/10
/// text-ink`, a 14 px icon; a `danger` one turns red when hovered.
pub struct CardButton<'a> {
    pub id: Id,
    pub icon: &'a str,
    /// The accessible name (`aria-label`).
    pub label: &'a str,
    /// The tooltip, and its shortcut.
    pub tip: &'a str,
    pub shortcut: Option<&'a str>,
    pub danger: bool,
}

/// A [`CardButton`] with its tooltip on `place`'s side.
pub fn card_button(ui: &Ui, b: &CardButton<'_>, place: Place) -> El {
    let hot = ui.hover(b.id);
    let fg = if b.danger {
        color::INK.mix(color::DANGER, hot)
    } else {
        color::INK.rgba()
    };
    let el = div()
        .id(b.id)
        .on(Action::Click)
        .p(4.0)
        .rounded(radius::SM)
        .bg(Rgba::TRANSPARENT.mix(vornui::theme::bright(0.10), hot))
        .ring(radius::SM)
        .role(Role::Button, b.label)
        .child(crate::lucide(b.icon, 14.0, 2.0, fg));
    tooltip_at(ui, el, b.id, b.tip, b.shortcut, place)
}

/// The ids and shortcut labels a [`card_actions`] cluster reports with.
pub struct CardActions<'a> {
    pub variant: CardVariant,
    pub more: Id,
    /// The session's pane buttons (Browse files, Terminals, ...), after More.
    pub panes: &'a [CardButton<'a>],
    /// Only a grid card minimizes.
    pub minimize: Id,
    /// Expand on a grid card, Collapse on the stage.
    pub toggle: Id,
    pub close: Id,
    /// The toggle's shortcut, as the platform writes it (`⌘O`, `⌘W`).
    pub toggle_key: &'a str,
    /// While the close is being confirmed: its Close and Cancel ids.
    pub confirming: Option<(Id, Id)>,
}

/// `CardActionCluster`: More actions, the pane buttons, Minimize (grid
/// cards only), Expand or Collapse, and Close with its confirmation,
/// `gap-0.5`.
pub fn card_actions(ui: &Ui, a: &CardActions<'_>) -> El {
    card_actions_with(ui, a, [])
}

/// [`card_actions`] with `extra` buttons after the pane buttons, where the
/// card's browser and device toggles sit.
pub fn card_actions_with(ui: &Ui, a: &CardActions<'_>, extra: impl IntoIterator<Item = El>) -> El {
    let place = a.variant.tip_place();
    let button = |b: CardButton<'_>| card_button(ui, &b, place);
    let more = button(CardButton {
        id: a.more,
        icon: "ellipsis",
        label: "More actions",
        tip: "More actions",
        shortcut: None,
        danger: false,
    });
    let toggle = match a.variant {
        CardVariant::Mini => CardButton {
            id: a.toggle,
            icon: "maximize-2",
            label: "Expand session",
            tip: "Expand",
            shortcut: Some(a.toggle_key),
            danger: false,
        },
        CardVariant::Focused => CardButton {
            id: a.toggle,
            icon: "minimize-2",
            label: "Collapse session",
            tip: "Collapse to grid",
            shortcut: Some(a.toggle_key),
            danger: false,
        },
    };
    let mut close = button(CardButton {
        id: a.close,
        icon: "x",
        label: "Close session",
        tip: "Close session",
        shortcut: None,
        danger: true,
    });
    if let Some((ok, cancel)) = a.confirming {
        close = confirm(
            ui,
            close,
            "Close this session?",
            (ok, "Close"),
            cancel,
            Tone::Danger,
        );
    }
    let minimize = (a.variant == CardVariant::Mini).then(|| {
        button(CardButton {
            id: a.minimize,
            icon: "minus",
            label: "Minimize session",
            tip: "Minimize",
            shortcut: None,
            danger: false,
        })
    });
    let mut row = div().row().items_center().gap(2.0).shrink0().child(more);
    for p in a.panes {
        row = row.child(card_button(ui, p, place));
    }
    row.children(extra)
        .child_if(minimize)
        .child(button(toggle))
        .child(close)
}
