//! Syntax colour against Shiki's: each fixture holds a file and the tokens
//! Shiki gives it, and the lexer has to agree on most of the
//! characters a person sees. The rate is per language, printed with
//! `--nocapture`; `HL_DIFF=<lang>` lists where they differ.

use serde_json::Value;
use vorn_app::highlight::{highlight, Color};

const TEXT: Color = (0xdbd7ca, 0xee);

fn shiki_color(v: &Value) -> Color {
    let Some(s) = v.as_str() else { return TEXT };
    let hex = s.trim_start_matches('#');
    let rgb = u32::from_str_radix(&hex[..6], 16).unwrap_or(0);
    let a = hex
        .get(6..8)
        .and_then(|a| u8::from_str_radix(a, 16).ok())
        .unwrap_or(0xff);
    (rgb, a)
}

/// The share of non-blank characters drawn in Shiki's colour.
fn agreement(name: &str) -> f64 {
    let path = format!(
        "{}/tests/fixtures/highlight/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect(&path)).expect("json");
    let code = fixture["code"].as_str().expect("code");
    let lang = fixture["lang"].as_str().expect("lang");
    let mine = highlight(code, lang);
    let color_at = |at: usize| {
        let i = mine.partition_point(|(r, _)| r.end <= at);
        mine.get(i)
            .filter(|(r, _)| r.start <= at)
            .map_or(TEXT, |(_, c)| *c)
    };
    let (mut same, mut all) = (0usize, 0usize);
    let mut at = 0;
    for (line, tokens) in code
        .split('\n')
        .zip(fixture["tokens"].as_array().expect("tokens"))
    {
        let mut col = at;
        for t in tokens.as_array().expect("line") {
            let text = t[0].as_str().unwrap_or("");
            let want = shiki_color(&t[1]);
            for (k, ch) in text.char_indices() {
                if !ch.is_whitespace() {
                    all += 1;
                    let got = color_at(col + k);
                    same += usize::from(got == want);
                    if got != want && std::env::var("HL_DIFF").as_deref() == Ok(name) {
                        let s =
                            &code[col.saturating_sub(12)..(col + text.len() + 8).min(code.len())];
                        eprintln!("{want:06x?} got {got:06x?} {text:?} in {s:?}");
                    }
                }
            }
            col += text.len();
        }
        at += line.len() + 1;
    }
    same as f64 / all.max(1) as f64
}

#[test]
fn the_lexer_draws_most_characters_as_shiki_does() {
    let floors = [
        ("typescript", 0.92),
        ("rust", 0.92),
        ("python", 0.88),
        ("bash", 0.85),
        ("yaml", 0.95),
        ("json", 0.98),
        ("markdown", 0.92),
        ("go", 0.92),
    ];
    let mut short = Vec::new();
    for (name, floor) in floors {
        let a = agreement(name);
        println!("{name}: {:.1}%", a * 100.0);
        if a < floor {
            short.push(format!("{name} {:.1}% < {:.0}%", a * 100.0, floor * 100.0));
        }
    }
    assert!(short.is_empty(), "{short:?}");
}
