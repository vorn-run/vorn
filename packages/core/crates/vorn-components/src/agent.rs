//! `AgentIcon` and `AgentStatusIcon`: each agent's own mark in its own
//! colours, a shell's terminal, and the running glyph while an agent works.

use std::collections::HashMap;

use vornui::theme::color;
use vornui::{image, rasterize_svg_rgba, El, Rgba, Ui};

use crate::status::Status;

/// The marks `AgentIcon` draws, by agent type (`agent-icons`).
pub const MARKS: [(&str, &str); 5] = [
    ("claude", include_str!("../assets/agents/claude.svg")),
    ("copilot", include_str!("../assets/agents/copilot.svg")),
    ("codex", include_str!("../assets/agents/codex.svg")),
    ("opencode", include_str!("../assets/agents/opencode.svg")),
    ("gemini", include_str!("../assets/agents/gemini.svg")),
];

/// How many pixels a mark is rasterized at: enough for the largest size
/// drawn (24) at 2x.
const RASTER: u32 = 48;

/// The marks registered as images with a [`Ui`], once per window.
#[derive(Debug, Default)]
pub struct AgentMarks(HashMap<&'static str, u32>);

impl AgentMarks {
    /// Registers every mark that is not registered yet; a mark that does
    /// not rasterize costs only itself, falling back to the terminal.
    pub fn load(&mut self, ui: &mut Ui) {
        if !self.0.is_empty() {
            return;
        }
        for (name, svg) in MARKS {
            if let Some(id) = rasterize_svg_rgba(svg, RASTER, RASTER)
                .and_then(|rgba| ui.image(RASTER, RASTER, &rgba))
            {
                self.0.insert(name, id);
            }
        }
    }

    /// The image of `agent`'s mark.
    pub fn get(&self, agent: &str) -> Option<u32> {
        self.0.get(agent).copied()
    }
}

/// `AgentIcon`: `agent`'s mark at `size`, or a shell's (and any unknown
/// agent's) terminal, `strokeWidth 1.5` in gray-400.
pub fn agent_icon(marks: &AgentMarks, agent: &str, size: f32) -> El {
    match marks.get(agent) {
        Some(id) => {
            let (g, tint) = cell_mark(agent);
            image(id, size, size).glyph(g).glyph_tint(tint)
        }
        None => crate::lucide("terminal", size, 1.5, color::GRAY_400),
    }
}

/// An agent's mark on a cells surface: one character in the mark's own colour.
fn cell_mark(agent: &str) -> (&'static str, Rgba) {
    let g = match agent {
        "claude" => "✳",
        "copilot" => "◉",
        "codex" => "◎",
        "opencode" => "▣",
        "gemini" => "✦",
        _ => "›",
    };
    let tint = MARKS
        .iter()
        .find(|(n, _)| *n == agent)
        .and_then(|(_, svg)| first_fill(svg))
        .unwrap_or(color::INK);
    (g, tint)
}

/// A mark's first `fill="#rrggbb"` (or `#rgb`).
fn first_fill(svg: &str) -> Option<Rgba> {
    let at = svg.find("fill=\"#")? + 7;
    let hex = &svg[at..at + svg[at..].find('"')?];
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex.to_owned(),
        _ => return None,
    };
    u32::from_str_radix(&full, 16).ok().map(Rgba::hex)
}

/// `AgentStatusIcon`: the running glyph in `white/85` while an agent
/// works, else its mark. A shell is always its terminal. `alpha` dims the
/// whole icon (a card that is not selected).
pub fn status_icon(
    ui: &Ui,
    marks: &AgentMarks,
    agent: &str,
    status: Status,
    size: f32,
    alpha: f32,
) -> El {
    if agent != "shell" && status == Status::Running {
        return crate::glyph::running(ui, size, color::WHITE.alpha(0.85 * alpha));
    }
    agent_icon(marks, agent, size).opacity(alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_keeps_its_colour_in_a_terminal() {
        assert_eq!(cell_mark("claude"), ("✳", Rgba::hex(0xd97757)));
        assert_eq!(cell_mark("codex").1, Rgba::hex(0xffffff));
        assert_eq!(cell_mark("shell"), ("›", color::INK));
    }
}
