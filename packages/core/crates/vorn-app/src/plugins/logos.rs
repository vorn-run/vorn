//! Extensions' logos, rasterized once per window and logo: a pack's logo
//! file (its dark variant on a dark surface), else its agent's mark.

use std::collections::HashMap;

use vornui::{rasterize_svg_rgba, Ui};

use super::Listing;

/// Pixels a logo is rasterized at: the largest drawn (64) at 2x.
const RASTER: u32 = 128;

/// The logos registered as images with a [`Ui`], by extension id.
#[derive(Debug, Default)]
pub struct Logos(HashMap<String, (String, Option<u32>)>);

impl Logos {
    /// Registers each `(id, svg)` whose SVG changed; one that does not
    /// rasterize costs only its image, so the row draws its fallback.
    pub fn load<'a>(&mut self, ui: &mut Ui, logos: impl IntoIterator<Item = (&'a str, &'a str)>) {
        for (id, svg) in logos {
            if self.0.get(id).is_some_and(|(had, _)| had == svg) {
                continue;
            }
            let image = rasterize_svg_rgba(svg, RASTER, RASTER)
                .and_then(|rgba| ui.image(RASTER, RASTER, &rgba));
            self.0.insert(id.to_owned(), (svg.to_owned(), image));
        }
    }

    /// The image of extension `id`'s logo.
    pub fn get(&self, id: &str) -> Option<u32> {
        self.0.get(id).and_then(|(_, image)| *image)
    }
}

/// The SVG to draw for `l`: its dark logo on a dark surface, its logo, else
/// the mark of the agent it adds (`agent_mark`).
pub fn svg<'a>(l: &'a Listing, dark: bool, agent_mark: Option<&'a str>) -> Option<&'a str> {
    dark.then_some(l.logo_dark.as_deref())
        .flatten()
        .or(l.logo.as_deref())
        .or(agent_mark)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(logo: Option<&str>, dark: Option<&str>) -> Listing {
        let mut p = crate::plugins::Plugins {
            packs: vec![serde_json::json!({
                "id": "x", "name": "X", "version": "1.0.0", "logo": logo, "logoDark": dark,
            })],
            ..Default::default()
        };
        p.rebuild();
        p.listings.remove(0)
    }

    #[test]
    fn prefers_the_dark_logo_on_dark_then_the_logo_then_the_agent_mark() {
        let both = listing(Some("<svg light/>"), Some("<svg dark/>"));
        assert_eq!(svg(&both, true, None), Some("<svg dark/>"));
        assert_eq!(svg(&both, false, None), Some("<svg light/>"));
        let light = listing(Some("<svg light/>"), None);
        assert_eq!(svg(&light, true, None), Some("<svg light/>"));
        let none = listing(None, None);
        assert_eq!(svg(&none, true, Some("<svg agent/>")), Some("<svg agent/>"));
        assert_eq!(svg(&none, false, None), None);
    }
}
