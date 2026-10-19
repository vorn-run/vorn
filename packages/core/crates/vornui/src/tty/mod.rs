//! The terminal UI drawn as pixels: where the terminal shows kitty
//! graphics, each frame the CPU renderer draws (the desktop's scene, at
//! the terminal's own pixel size) is sent as images, only the parts that
//! changed, and input is read at pixel precision. Elsewhere the Cells
//! backend draws text cells as before.

pub mod input;
pub mod kitty;
pub mod present;
mod screen;

pub use screen::{probe, Caps, PixelTerminal, Probe};

/// How the terminal UI draws: the choice in Settings and `vorn --drawing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Drawing {
    /// Pixels where the terminal shows them, text cells elsewhere.
    #[default]
    Auto,
    Pixels,
    Text,
}

impl Drawing {
    pub fn as_str(self) -> &'static str {
        match self {
            Drawing::Auto => "auto",
            Drawing::Pixels => "pixels",
            Drawing::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Option<Drawing> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "automatic" => Some(Drawing::Auto),
            "pixels" | "pixel" => Some(Drawing::Pixels),
            "text" | "cells" => Some(Drawing::Text),
            _ => None,
        }
    }

    /// Whether to draw pixels on a terminal that can (`supported`) or not:
    /// Pixels where they cannot be shown falls back to text.
    pub fn pixels(self, supported: bool) -> bool {
        self != Drawing::Text && supported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_round_trips_and_falls_back_to_text() {
        for d in [Drawing::Auto, Drawing::Pixels, Drawing::Text] {
            assert_eq!(Drawing::parse(d.as_str()), Some(d));
        }
        assert_eq!(Drawing::parse(" Automatic "), Some(Drawing::Auto));
        assert_eq!(Drawing::parse("bitmap"), None);
        assert!(Drawing::Auto.pixels(true));
        assert!(!Drawing::Pixels.pixels(false));
        assert!(!Drawing::Text.pixels(true));
    }
}
