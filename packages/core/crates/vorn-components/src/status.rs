//! A session's status (`AgentStatus`) and its dot (`STATUS_DOT`): only a
//! waiting session takes the accent, and only a running one moves.

use vornui::theme::color;
use vornui::{div, El, Rgba, Ui};

/// What a session is doing, as vornd reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Running,
    Waiting,
    Idle,
    Error,
}

impl Status {
    /// vornd's word for it; anything unknown reads as idle, as the
    /// renderer's maps fall back.
    pub fn parse(s: &str) -> Status {
        match s {
            "running" => Status::Running,
            "waiting" => Status::Waiting,
            "error" => Status::Error,
            _ => Status::Idle,
        }
    }

    /// `STATUS_LABEL`.
    pub fn label(self) -> &'static str {
        match self {
            Status::Running => "Running",
            Status::Waiting => "Waiting",
            Status::Idle => "Idle",
            Status::Error => "Error",
        }
    }

    /// `STATUS_DOT`: live ink, blocked bronzo, idle ghost, broken danger.
    pub fn dot(self) -> Rgba {
        match self {
            Status::Running => color::INK,
            Status::Waiting => color::BRONZO,
            Status::Idle => color::INK_GHOST,
            Status::Error => color::DANGER,
        }
    }
}

/// The 6 px status dot; a running one pulses (`animate-pulse`).
pub fn status_dot(ui: &Ui, status: Status) -> El {
    let fill = status.dot();
    let fill = if status == Status::Running {
        fill.alpha(crate::glyph::pulse(ui))
    } else {
        fill
    };
    div().size(6.0, 6.0).rounded(3.0).shrink0().bg(fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_waiting_takes_the_accent() {
        for s in [Status::Running, Status::Idle, Status::Error] {
            assert_ne!(s.dot(), color::BRONZO, "{s:?}");
        }
        assert_eq!(Status::Waiting.dot(), color::BRONZO);
    }

    #[test]
    fn unknown_words_read_as_idle() {
        assert_eq!(Status::parse("running"), Status::Running);
        assert_eq!(Status::parse("exited"), Status::Idle);
    }
}
