//! One colour as a theme writes it: 8-bit sRGB with alpha, read from and
//! written as `#rrggbb` or `#rrggbbaa`, and the sRGB, linear and OKLCH
//! arithmetic the derivation and the contrast check need.

use std::fmt;

/// An sRGB colour with straight (not premultiplied) alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const BLACK: Rgba8 = Rgba8::rgb(0x000000);
    pub const WHITE: Rgba8 = Rgba8::rgb(0xffffff);

    /// `0xRRGGBB`, opaque.
    pub const fn rgb(c: u32) -> Rgba8 {
        Rgba8 {
            r: (c >> 16) as u8,
            g: (c >> 8) as u8,
            b: c as u8,
            a: 255,
        }
    }

    /// The same colour at `alpha` (0 to 1).
    pub fn with_alpha(self, alpha: f32) -> Rgba8 {
        Rgba8 {
            a: unit_to_u8(alpha),
            ..self
        }
    }

    /// `0xRRGGBB`, alpha dropped.
    pub const fn to_u32(self) -> u32 {
        (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub fn alpha(self) -> f32 {
        f32::from(self.a) / 255.0
    }

    /// Reads `#rgb`, `#rrggbb` or `#rrggbbaa`.
    pub fn parse(text: &str) -> Option<Rgba8> {
        let hex = text.strip_prefix('#')?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let n = u32::from_str_radix(hex, 16).ok()?;
        match hex.len() {
            3 => {
                let dup = |v: u32| (v & 0xf) * 0x11;
                Some(Rgba8::rgb(dup(n >> 8) << 16 | dup(n >> 4) << 8 | dup(n)))
            }
            6 => Some(Rgba8::rgb(n)),
            8 => Some(Rgba8 {
                a: n as u8,
                ..Rgba8::rgb(n >> 8)
            }),
            _ => None,
        }
    }

    /// `self` moved `t` (0 to 1) of the way to `other`, alpha included.
    pub fn mix(self, other: Rgba8, t: f32) -> Rgba8 {
        let t = t.clamp(0.0, 1.0);
        let lerp =
            |a: u8, b: u8| unit_to_u8((f32::from(a) + (f32::from(b) - f32::from(a)) * t) / 255.0);
        Rgba8 {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }

    /// `self` painted over the opaque `under`, as a browser blends: in sRGB.
    pub fn over(self, under: Rgba8) -> Rgba8 {
        Rgba8 {
            a: 255,
            ..under.mix(Rgba8 { a: 255, ..self }, self.alpha())
        }
    }

    /// WCAG 2's relative luminance of the opaque colour.
    pub fn luminance(self) -> f32 {
        let [r, g, b] = self.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Linear-light channels, 0 to 1.
    pub fn linear(self) -> [f32; 3] {
        [self.r, self.g, self.b].map(|c| to_linear(f32::from(c) / 255.0))
    }

    pub fn from_linear([r, g, b]: [f32; 3], a: u8) -> Rgba8 {
        Rgba8 {
            r: unit_to_u8(from_linear(r)),
            g: unit_to_u8(from_linear(g)),
            b: unit_to_u8(from_linear(b)),
            a,
        }
    }

    /// Lightness (0 to 1), chroma and hue (degrees) in OKLCH.
    pub fn oklch(self) -> Oklch {
        let [r, g, b] = self.linear();
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
        let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
        let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
        Oklch {
            l: lightness,
            c: a.hypot(b),
            h: b.atan2(a).to_degrees().rem_euclid(360.0),
        }
    }
}

impl fmt::Display for Rgba8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:06x}", self.to_u32())?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

/// A colour in OKLCH: perceptual lightness, chroma and hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

impl Oklch {
    /// Back to sRGB, or `None` when the colour lies outside its gamut.
    pub fn to_rgb(self, alpha: u8) -> Option<Rgba8> {
        let (a, b) = (
            self.c * self.h.to_radians().cos(),
            self.c * self.h.to_radians().sin(),
        );
        let l = self.l + 0.396_337_78 * a + 0.215_803_76 * b;
        let m = self.l - 0.105_561_346 * a - 0.063_854_17 * b;
        let s = self.l - 0.089_484_18 * a - 1.291_485_5 * b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        let lin = [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ];
        const EPS: f32 = 1e-4;
        lin.iter()
            .all(|c| (-EPS..=1.0 + EPS).contains(c))
            .then(|| Rgba8::from_linear(lin.map(|c| c.clamp(0.0, 1.0)), alpha))
    }

    /// The nearest colour in gamut at this lightness and hue: chroma is
    /// given up first, so the hue stays what it was.
    pub fn to_rgb_clipped(self, alpha: u8) -> Rgba8 {
        if let Some(c) = self.to_rgb(alpha) {
            return c;
        }
        let (mut lo, mut hi) = (0.0f32, self.c);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if (Oklch { c: mid, ..self }).to_rgb(alpha).is_some() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (Oklch { c: lo, ..self })
            .to_rgb(alpha)
            .unwrap_or_else(|| Rgba8::from_linear([self.l.clamp(0.0, 1.0).powi(3); 3], alpha))
    }
}

fn unit_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_hex() {
        assert_eq!(Rgba8::parse("#c9972a"), Some(Rgba8::rgb(0xc9972a)));
        assert_eq!(Rgba8::parse("#fa0"), Some(Rgba8::rgb(0xffaa00)));
        assert_eq!(Rgba8::parse("#ffffff80").map(|c| c.a), Some(0x80));
        for bad in ["c9972a", "#c9972", "#gggggg", "#+12345", ""] {
            assert_eq!(Rgba8::parse(bad), None, "{bad}");
        }
        assert_eq!(Rgba8::rgb(0x0d0d0f).to_string(), "#0d0d0f");
        assert_eq!(Rgba8::WHITE.with_alpha(0.5).to_string(), "#ffffff80");
    }

    #[test]
    fn oklch_round_trips_in_gamut() {
        for c in [0xc9972a, 0x0d0d0f, 0xffffff, 0x268bd2, 0xdc322f, 0x000000] {
            let rgb = Rgba8::rgb(c);
            let back = rgb.oklch().to_rgb(255).expect("in gamut");
            let d = |a: u8, b: u8| a.abs_diff(b);
            assert!(
                d(rgb.r, back.r) <= 1 && d(rgb.g, back.g) <= 1 && d(rgb.b, back.b) <= 1,
                "{rgb} -> {back}"
            );
        }
    }

    #[test]
    fn luminance_matches_wcag() {
        assert!((Rgba8::WHITE.luminance() - 1.0).abs() < 1e-4);
        assert!(Rgba8::BLACK.luminance().abs() < 1e-6);
        // #777 is the classic 4.48:1 on white.
        let grey = Rgba8::rgb(0x777777).luminance();
        assert!(((1.05 / (grey + 0.05)) - 4.48).abs() < 0.01);
    }
}
