//! `AgentIcon` and `AgentStatusIcon`: each agent's own mark in its own
//! colours, a shell's terminal, and the running glyph while an agent works.
//! The marks come from the agents' adapters, as SVG text and one character
//! for a cells surface.

use std::collections::HashMap;

use vornui::theme::color;
use vornui::{image, rasterize_svg_rgba, El, Rgba, Ui};

use crate::status::Status;

/// How many pixels a mark is rasterized at: enough for the largest size
/// drawn (24) at 2x.
const RASTER: u32 = 48;

/// One registered mark: its image, its character and its colour.
#[derive(Debug, Clone)]
struct Mark {
    image: Option<u32>,
    glyph: String,
    tint: Rgba,
    /// The SVG it was made from, so a changed mark is made again.
    svg: String,
}

/// The marks registered as images with a [`Ui`], once per window and agent.
#[derive(Debug, Default)]
pub struct AgentMarks(HashMap<String, Mark>);

impl AgentMarks {
    /// Registers each of `marks` (agent type, SVG, character) not registered
    /// yet; a mark that does not rasterize costs only its image, falling
    /// back to the terminal.
    pub fn load<'a>(
        &mut self,
        ui: &mut Ui,
        marks: impl IntoIterator<Item = (&'a str, &'a str, Option<&'a str>)>,
    ) {
        for (name, svg, glyph) in marks {
            if self.0.get(name).is_some_and(|m| m.svg == svg) {
                continue;
            }
            let image = rasterize_svg_rgba(svg, RASTER, RASTER)
                .and_then(|rgba| ui.image(RASTER, RASTER, &rgba));
            self.0.insert(
                name.to_owned(),
                Mark {
                    image,
                    glyph: glyph.unwrap_or("›").to_owned(),
                    tint: first_fill(svg).unwrap_or(color::INK),
                    svg: svg.to_owned(),
                },
            );
        }
    }

    /// The image of `agent`'s mark.
    pub fn get(&self, agent: &str) -> Option<u32> {
        self.0.get(agent).and_then(|m| m.image)
    }

    /// `agent`'s mark on a cells surface: one character in the mark's own colour.
    fn cell_mark(&self, agent: &str) -> (&str, Rgba) {
        self.0
            .get(agent)
            .map_or(("›", color::INK), |m| (m.glyph.as_str(), m.tint))
    }
}

/// `AgentIcon`: `agent`'s mark at `size`, or a shell's (and any unknown
/// agent's) terminal, `strokeWidth 1.5` in gray-400.
pub fn agent_icon(marks: &AgentMarks, agent: &str, size: f32) -> El {
    match marks.get(agent) {
        Some(id) => {
            let (g, tint) = marks.cell_mark(agent);
            image(id, size, size).glyph(g).glyph_tint(tint)
        }
        None => crate::lucide("terminal", size, 1.5, color::GRAY_400),
    }
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
        assert_eq!(
            first_fill(r##"<svg><path fill="#d97757" d="M0 0"/></svg>"##),
            Some(Rgba::hex(0xd97757))
        );
        assert_eq!(
            first_fill(r##"<path fill="#fff"/>"##),
            Some(Rgba::hex(0xffffff))
        );
        assert_eq!(first_fill("<svg/>"), None);
        assert_eq!(AgentMarks::default().cell_mark("shell"), ("›", color::INK));
    }
}
