//! The video primitive's host: the iOS simulators a `video` part shows,
//! driven through `simctl` for `video:*` calls, their screens turned into
//! frames the pane draws itself, and the zoom and bezel those are drawn
//! with. Simulators exist only on macOS; elsewhere the list is empty and a
//! boot says why. Which session holds which simulator is the Device
//! extension's to decide, not this host's.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::Value;

/// One simulator, as `video:list` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Sim {
    pub udid: String,
    pub name: String,
    pub runtime: String,
    pub booted: bool,
}

/// `com.apple.CoreSimulator.SimRuntime.iOS-26-2` reads
/// `iOS 26.2`.
pub fn format_runtime(id: &str) -> String {
    let bare = id
        .strip_prefix("com.apple.CoreSimulator.SimRuntime.")
        .unwrap_or(id);
    match bare.split_once('-') {
        Some((name, version)) => format!("{name} {}", version.replace('-', ".")),
        None => bare.to_owned(),
    }
}

/// The iOS simulators in `simctl list devices available -j`'s output;
/// watchOS and tvOS ones are left out.
pub fn parse_simctl(json: &Value) -> Vec<Sim> {
    let Some(Value::Object(runtimes)) = json.get("devices") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (runtime, list) in runtimes {
        if !runtime.contains("iOS") {
            continue;
        }
        for d in list.as_array().into_iter().flatten() {
            if d.get("isAvailable").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let s = |k: &str| {
                d.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            out.push(Sim {
                udid: s("udid"),
                name: s("name"),
                runtime: format_runtime(runtime),
                booted: s("state") == "Booted",
            });
        }
    }
    out
}

/// Why a device call failed, said as what to do about it.
#[cfg(not(target_arch = "wasm32"))]
fn explain(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("xcode-select")
        || lower.contains("command line tools")
        || lower.contains("developerdirectory")
    {
        return "Xcode is not selected \u{2014} the Command Line Tools are, and they carry no simulators.\nThis needs a password, so you have to run it yourself:\n  sudo xcode-select -s /Applications/Xcode.app/Contents/Developer".to_owned();
    }
    if lower.contains("no runtime") || lower.contains("runtime profile") {
        return format!(
            "{msg}\nInstall an iOS runtime: Xcode \u{2192} Settings \u{2192} Components \u{2192} iOS Simulator."
        );
    }
    msg.to_owned()
}

/// Why a browser shows no device.
pub const IN_BROWSER: &str =
    "Simulators run beside the desktop app. Open this session in Vorn on the Mac to see its device.";

#[cfg(target_arch = "wasm32")]
fn simctl(_args: &[&str]) -> Result<String, String> {
    Err(IN_BROWSER.to_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn simctl(args: &[&str]) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("Simulators need macOS with Xcode.".to_owned());
    }
    let out = vorn_spawn::command("xcrun")
        .arg("simctl")
        .args(args)
        .output()
        .map_err(|e| explain(&e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(explain(String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Every simulator on this machine. Empty off macOS.
pub fn list() -> Result<Vec<Sim>, String> {
    if cfg!(target_arch = "wasm32") {
        return Err(IN_BROWSER.to_owned());
    }
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    let raw = simctl(&["list", "devices", "available", "-j"])?;
    let json: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(parse_simctl(&json))
}

/// Starts `udid` when it is shut down, and waits until it is up. Its name,
/// and whether this started it. Blocks for as long as the boot takes, so it
/// runs off the UI thread.
pub fn boot(udid: &str) -> Result<(String, bool), String> {
    let sims = list()?;
    let Some(d) = sims.iter().find(|d| d.udid == udid) else {
        return Err(format!(
            "No simulator with udid {udid}. Call device_list to see what is available."
        ));
    };
    if !d.booted {
        simctl(&["boot", udid])?;
        simctl(&["bootstatus", udid, "-b"])?;
    }
    Ok((d.name.clone(), !d.booted))
}

/// Shuts the simulator down.
pub fn shutdown(udid: &str) -> Result<(), String> {
    simctl(&["shutdown", udid]).map(drop)
}

/// Starts the app `bundle`, bringing it forward when it already runs.
pub fn launch(udid: &str, bundle: &str) -> Result<(), String> {
    simctl(&["launch", "--terminate-running-process", udid, bundle]).map(drop)
}

pub fn terminate(udid: &str, bundle: &str) -> Result<(), String> {
    simctl(&["terminate", udid, bundle]).map(drop)
}

pub fn open_url(udid: &str, url: &str) -> Result<(), String> {
    simctl(&["openurl", udid, url]).map(drop)
}

/// Installs the `.app` at `path`.
pub fn install(udid: &str, path: &str) -> Result<(), String> {
    simctl(&["install", udid, path]).map(drop)
}

/// A PNG of the simulator's screen.
pub fn screenshot(udid: &str) -> Result<Vec<u8>, String> {
    static SHOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SHOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("vorn-device-{}-{udid}-{n}.png", std::process::id()));
    let p = path.to_string_lossy().into_owned();
    simctl(&["io", udid, "screenshot", "--type=png", &p])?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    bytes
}

/// The device frame's zoom bounds and step.
pub const ZOOM_MIN: f64 = 0.25;
pub const ZOOM_MAX: f64 = 4.0;
const ZOOM_STEP: f64 = 1.25;

/// One step in or out, rounded as `toFixed(3)` and clamped.
pub fn stepped_zoom(current: f64, inward: bool) -> f64 {
    let next = if inward {
        current * ZOOM_STEP
    } else {
        current / ZOOM_STEP
    };
    to_fixed3(next).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// `Number(x.toFixed(3))` for a positive `x`: rounded from its exact decimal
/// value with ties going up. Scaling by 1000 first would round the product
/// instead, which differs (`0.826 * 1.25` is just under `1.0325`).
fn to_fixed3(x: f64) -> f64 {
    // `{:.60}` prints the exact decimal expansion of any zoom in range.
    let exact = format!("{x:.60}");
    let Some((whole, frac)) = exact.split_once('.') else {
        return x;
    };
    let Ok(thousandths) = format!("{whole}{}", &frac[..3]).parse::<u64>() else {
        return x;
    };
    let up = u64::from(frac.as_bytes()[3] >= b'5');
    // An integer over 1000 is the double nearest that decimal, as `Number` reads it.
    (thousandths + up) as f64 / 1000.0
}

/// Image pixels per point for a screenshot `w`×`h`: tall phones with
/// 1000+ pixels across are 3x, every other simulator 2x.
pub fn point_scale(w: u32, h: u32) -> f64 {
    let (short, long) = (w.min(h).max(1), w.max(h));
    if short >= 1000 && f64::from(long) / f64::from(short) > 1.9 {
        3.0
    } else {
        2.0
    }
}

/// Space kept between the frame and the pane's edge (`GUTTER`).
const GUTTER: f64 = 8.0;
/// Long over short at which a screen is a phone's (`PHONE_ASPECT`).
const PHONE_ASPECT: f64 = 1.8;

fn is_phone(points: (f64, f64)) -> bool {
    let (long, short) = (points.0.max(points.1), points.0.min(points.1));
    short <= 0.0 || long / short >= PHONE_ASPECT
}

/// The drawn frame's width, from the pane so it keeps its
/// weight while the device zooms.
pub fn bezel_thickness(area: (f64, f64), points: (f64, f64)) -> f64 {
    let k = if is_phone(points) { 0.035 } else { 0.028 };
    (area.0.min(area.1) * k).round().clamp(6.0, 16.0)
}

/// The scale at which the screen and its frame fit `area`; 1
/// before either is known.
pub fn fit_scale(points: (f64, f64), area: (f64, f64)) -> f64 {
    let inset = 2.0 * bezel_thickness(area, points) + 2.0 * GUTTER;
    let (w, h) = (area.0 - inset, area.1 - inset);
    if points.0 <= 0.0 || points.1 <= 0.0 || w <= 0.0 || h <= 0.0 {
        return 1.0;
    }
    (w / points.0).min(h / points.1)
}

/// The plain frame's outer corner radius for a screen drawn `w`×`h`.
pub fn outer_radius(drawn: (f64, f64), points: (f64, f64)) -> f64 {
    let k = if is_phone(points) { 0.085 } else { 0.035 };
    (drawn.0.min(drawn.1) * k).round()
}

/// The device's name made safe for a file, and when, as
/// `iso` (`toISOString`) gives it.
pub fn screenshot_name(name: &str, iso: &str) -> String {
    let stamp = iso
        .get(..19)
        .unwrap_or(iso)
        .replace('T', " ")
        .replace(':', "-");
    // A run of anything else is one dash (`/[^\w.-]+/g`), none at either end.
    let mut slug = String::new();
    let mut gap = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.push(ch);
        } else {
            gap = true;
        }
    }
    let slug = slug.strip_prefix('-').unwrap_or(&slug);
    let slug = slug.strip_suffix('-').unwrap_or(slug);
    let slug = if slug.is_empty() { "device" } else { slug };
    format!("{slug} {stamp}.png")
}

/// A PNG's width and height, from its header.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    let be = |at: usize| png.get(at..at + 4)?.try_into().ok().map(u32::from_be_bytes);
    png.starts_with(b"\x89PNG\r\n\x1a\n")
        .then(|| Some((be(16)?, be(20)?)))
        .flatten()
}

/// Where a saved screenshot goes: Downloads, else home.
fn downloads() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let home = PathBuf::from(home);
    let down = home.join("Downloads");
    Some(if down.is_dir() { down } else { home })
}

/// Saves the screen of `udid`, named for `name` and now. Where it went.
pub fn save(udid: &str, name: &str) -> Result<PathBuf, String> {
    let png = screenshot(udid)?;
    let dir = downloads().ok_or("No folder to save the screenshot in.")?;
    let iso = crate::tasks::iso(crate::state::now_ms());
    let path = dir.join(screenshot_name(name, &iso));
    std::fs::write(&path, png).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The longest side a frame is kept at: the colour atlas takes no bigger
/// picture, and a pane never shows a phone larger.
const FRAME_SIDE: u32 = 1024;

/// A screen as the pane draws it: straight RGBA, shrunk to fit
/// [`FRAME_SIDE`], and the screen's size in points.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// The picture's key in the atlas, new for each frame.
    pub key: u64,
    pub size: (u32, u32),
    pub rgba: Vec<u8>,
    pub points: (f64, f64),
}

/// The atlas keys of frames, apart from the terminals' pictures.
const FRAME_KEYS: u64 = 1 << 62;

/// Decodes a screenshot into a frame: shrunk by a whole factor with a box
/// filter, once, on the thread that took it.
pub fn frame(png: &[u8]) -> Result<Frame, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let (w, h, rgba) = crate::ui::decode_png(png)?;
    let k = w.max(h).div_ceil(FRAME_SIDE).max(1);
    let scale = point_scale(w, h);
    Ok(Frame {
        key: FRAME_KEYS | NEXT.fetch_add(1, Ordering::Relaxed),
        size: (w / k, h / k),
        rgba: shrink(&rgba, (w, h), k),
        points: (f64::from(w) / scale, f64::from(h) / scale),
    })
}

/// `rgba` (`w`×`h`) shrunk `k` times each way, each pixel the mean of its
/// `k`×`k` block; a ragged edge is dropped.
fn shrink(rgba: &[u8], (w, h): (u32, u32), k: u32) -> Vec<u8> {
    if k <= 1 {
        return rgba.to_vec();
    }
    let (w, h, k) = (w as usize, h as usize, k as usize);
    let (ow, oh) = (w / k, h / k);
    let mut out = Vec::with_capacity(ow * oh * 4);
    let n = (k * k) as u32;
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = [0u32; 4];
            for y in oy * k..oy * k + k {
                let row = &rgba[(y * w + ox * k) * 4..(y * w + ox * k + k) * 4];
                for px in row.chunks_exact(4) {
                    for (s, &b) in sum.iter_mut().zip(px) {
                        *s += u32::from(b);
                    }
                }
            }
            out.extend(sum.map(|s| (s / n) as u8));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn screenshot_names_are_safe_file_names() {
        let at = "2026-03-04T05:06:07.890Z";
        assert_eq!(
            screenshot_name("iPhone 16 Pro (2)", at),
            "iPhone-16-Pro-2 2026-03-04 05-06-07.png"
        );
        assert_eq!(screenshot_name("  ", at), "device 2026-03-04 05-06-07.png");
    }

    #[test]
    fn zoom_steps_round_and_clamp() {
        assert_eq!(stepped_zoom(1.0, true), 1.25);
        assert_eq!(stepped_zoom(1.0, false), 0.8);
        assert_eq!(stepped_zoom(0.3, false), ZOOM_MIN);
        assert_eq!(stepped_zoom(3.9, true), ZOOM_MAX);
        assert_eq!(stepped_zoom(0.826, true), 1.032);
        assert_eq!(stepped_zoom(ZOOM_MIN, true), 0.313);
        assert_eq!(point_scale(1206, 2622), 3.0);
        assert_eq!(point_scale(750, 1334), 2.0);
        assert_eq!(point_scale(2064, 2752), 2.0);
        // A 400-wide pane: a 14px frame and 8px gutters each side leave 356.
        assert_eq!(bezel_thickness((400.0, 900.0), (390.0, 844.0)), 14.0);
        assert_eq!(fit_scale((356.0, 100.0), (400.0, 900.0)), 1.0);
        assert_eq!(fit_scale((390.0, 844.0), (0.0, 0.0)), 1.0);
        assert_eq!(bezel_thickness((100.0, 100.0), (1024.0, 1366.0)), 6.0);
        assert_eq!(outer_radius((390.0, 844.0), (390.0, 844.0)), 33.0);
    }

    #[test]
    fn png_size_reads_the_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&[0, 0, 1, 0x2c, 0, 0, 2, 0x58]);
        assert_eq!(png_size(&png), Some((300, 600)));
        assert_eq!(png_size(b"GIF89a"), None);
    }

    #[test]
    fn simctl_lists_only_available_ios_devices() {
        let raw = json!({"devices": {
            "com.apple.CoreSimulator.SimRuntime.iOS-26-2": [
                {"udid": "u1", "name": "iPhone", "state": "Booted", "isAvailable": true},
                {"udid": "u2", "name": "Old", "state": "Shutdown", "isAvailable": false}
            ],
            "com.apple.CoreSimulator.SimRuntime.watchOS-11-0": [
                {"udid": "w", "name": "Watch", "state": "Shutdown", "isAvailable": true}
            ]
        }});
        let sim = Sim {
            udid: "u1".into(),
            name: "iPhone".into(),
            runtime: "iOS 26.2".into(),
            booted: true,
        };
        assert_eq!(parse_simctl(&raw), [sim]);
    }

    #[test]
    fn a_frame_shrinks_by_whole_blocks() {
        // 2x2 blocks of a 4x2 image: two pixels, each its block's mean.
        let px = |v: u8| [v, v, v, 255];
        let rgba: Vec<u8> = [0, 10, 100, 110, 20, 30, 120, 130]
            .into_iter()
            .flat_map(px)
            .collect();
        assert_eq!(shrink(&rgba, (4, 2), 2), [px(15), px(115)].concat());
        assert_eq!(shrink(&rgba, (4, 2), 1), rgba);
    }
}
