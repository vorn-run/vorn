//! iOS simulators for the device pane: what `simctl` lists, the picker's
//! order (`device-picker-list.ts`), when a card offers the device button
//! (`device-affordance.ts`), and claiming one for a session. Simulators
//! exist only on macOS; elsewhere the list is empty and a claim says why.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

/// One simulator, as `device:list` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub runtime: String,
    pub booted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
}

/// A device in the picker: whether this session may take it, and what
/// tells it apart from another of the same name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub device: Device,
    pub selectable: bool,
    pub subtitle: Option<String>,
}

/// `isSelectable`: free, or already this session's.
pub fn is_selectable(d: &Device, session: &str) -> bool {
    d.claimed_by.as_deref().is_none_or(|c| c == session)
}

/// `orderDevices`: selectable first, then booted, then by name and runtime;
/// a subtitle only where the name alone is ambiguous.
pub fn order_devices(devices: &[Device], session: &str) -> Vec<Listed> {
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    let mut by_both: HashMap<(&str, &str), usize> = HashMap::new();
    for d in devices {
        *by_name.entry(&d.name).or_default() += 1;
        *by_both.entry((&d.name, &d.runtime)).or_default() += 1;
    }
    let mut out: Vec<Listed> = devices
        .iter()
        .map(|d| {
            let subtitle = (by_name[d.name.as_str()] > 1).then(|| {
                if by_both[&(d.name.as_str(), d.runtime.as_str())] > 1 {
                    let head: String = d.udid.chars().take(8).collect();
                    format!("{} \u{b7} {head}", d.runtime)
                } else {
                    d.runtime.clone()
                }
            });
            Listed {
                device: d.clone(),
                selectable: is_selectable(d, session),
                subtitle,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.selectable
            .cmp(&a.selectable)
            .then(b.device.booted.cmp(&a.device.booted))
            .then_with(|| locale_cmp(&a.device.name, &b.device.name))
            .then_with(|| locale_cmp(&a.device.runtime, &b.device.runtime))
    });
    out
}

/// Close to `localeCompare`: case folds first, so "iPad" sorts with "IPAD".
fn locale_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
}

/// `shouldShowDeviceButton`: an open pane always keeps its button; a shell
/// never offers one; otherwise only a mobile project does.
pub fn should_show_button(mobile: Option<bool>, has_pane: bool, shell: bool) -> bool {
    if has_pane {
        return true;
    }
    !shell && mobile == Some(true)
}

/// `formatRuntime`: `com.apple.CoreSimulator.SimRuntime.iOS-26-2` reads
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
/// watchOS and tvOS ones are left out, as the renderer leaves them.
pub fn parse_simctl(json: &Value, claims: &HashMap<String, String>) -> Vec<Device> {
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
            let udid = s("udid");
            out.push(Device {
                claimed_by: claims.get(&udid).cloned(),
                name: s("name"),
                runtime: format_runtime(runtime),
                booted: s("state") == "Booted",
                udid,
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

/// Every simulator on this machine, with who holds it. Empty off macOS.
pub fn list(claims: &HashMap<String, String>) -> Result<Vec<Device>, String> {
    if cfg!(target_arch = "wasm32") {
        return Err(IN_BROWSER.to_owned());
    }
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    let raw = simctl(&["list", "devices", "available", "-j"])?;
    let json: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(parse_simctl(&json, claims))
}

/// What a claim did: the device and whether the app booted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub udid: String,
    pub name: String,
    pub booted_here: bool,
}

/// Takes `udid` for `session`, booting it when it is shut down. Blocks for
/// as long as the boot takes, so it runs off the UI thread.
pub fn claim(
    udid: &str,
    session: &str,
    claims: &HashMap<String, String>,
) -> Result<Claimed, String> {
    let devices = list(claims)?;
    let Some(d) = devices.iter().find(|d| d.udid == udid) else {
        return Err(format!(
            "No simulator with udid {udid}. Call device_list to see what is available."
        ));
    };
    if !is_selectable(d, session) {
        return Err(format!(
            "{} is in use by session {}.",
            d.name,
            d.claimed_by.as_deref().unwrap_or_default()
        ));
    }
    if !d.booted {
        simctl(&["boot", udid])?;
        simctl(&["bootstatus", udid, "-b"])?;
    }
    Ok(Claimed {
        udid: udid.to_owned(),
        name: d.name.clone(),
        booted_here: !d.booted,
    })
}

/// Shuts down a simulator the app booted; one the person booted stays up.
pub fn shutdown(udid: &str) {
    let _ = simctl(&["shutdown", udid]);
}

/// Starts the app `bundle`, bringing it forward when it already runs.
pub fn launch(udid: &str, bundle: &str) -> Result<(), String> {
    simctl(&["launch", "--terminate-running-process", udid, bundle])
        .map(drop)
        .map_err(|e| {
            let lower = e.to_lowercase();
            if ["not installed", "no such", "cannot find", "not found"]
                .iter()
                .any(|p| lower.contains(p))
            {
                format!("{bundle} is not installed on this device. Install it first.")
            } else {
                e
            }
        })
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

/// The device frame's zoom bounds and step (`device-bezel.ts`).
pub const ZOOM_MIN: f64 = 0.25;
pub const ZOOM_MAX: f64 = 4.0;
const ZOOM_STEP: f64 = 1.25;

/// `steppedZoom`: one step in or out, rounded as `toFixed(3)` and clamped.
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

/// `bezelThickness`: the drawn frame's width, from the pane so it keeps its
/// weight while the device zooms.
pub fn bezel_thickness(area: (f64, f64), points: (f64, f64)) -> f64 {
    let k = if is_phone(points) { 0.035 } else { 0.028 };
    (area.0.min(area.1) * k).round().clamp(6.0, 16.0)
}

/// `fitScale`: the scale at which the screen and its frame fit `area`; 1
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

/// `screenshotName`: the device's name made safe for a file, and when, as
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

/// Standard base64 with padding, for the PNGs the agent reads.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
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
    fn base64_pads_like_btoa() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
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

    fn dev(udid: &str, name: &str, runtime: &str, booted: bool, by: Option<&str>) -> Device {
        Device {
            udid: udid.into(),
            name: name.into(),
            runtime: runtime.into(),
            booted,
            claimed_by: by.map(Into::into),
        }
    }

    #[test]
    fn the_picker_puts_free_and_booted_devices_first() {
        let all = [
            dev("aaaaaaaa1", "iPhone 16", "iOS 18.0", false, None),
            dev("bbbbbbbb2", "iPhone 16", "iOS 18.0", false, None),
            dev("c", "iPhone 16", "iOS 17.5", true, None),
            dev("d", "iPad", "iOS 18.0", false, Some("other")),
            dev("e", "iPad mini", "iOS 18.0", false, Some("me")),
        ];
        let listed = order_devices(&all, "me");
        let order: Vec<&str> = listed.iter().map(|l| l.device.udid.as_str()).collect();
        assert_eq!(order, ["c", "e", "aaaaaaaa1", "bbbbbbbb2", "d"]);
        assert_eq!(listed[0].subtitle.as_deref(), Some("iOS 17.5"));
        assert_eq!(
            listed[2].subtitle.as_deref(),
            Some("iOS 18.0 \u{b7} aaaaaaaa")
        );
        assert_eq!(listed[1].subtitle, None);
        assert!(!listed[4].selectable);
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
        let claims = HashMap::from([("u1".to_owned(), "s".to_owned())]);
        assert_eq!(
            parse_simctl(&raw, &claims),
            [dev("u1", "iPhone", "iOS 26.2", true, Some("s"))]
        );
    }

    #[test]
    fn the_device_button_follows_the_pane_then_the_project() {
        assert!(should_show_button(None, true, true));
        assert!(!should_show_button(Some(true), false, true));
        assert!(should_show_button(Some(true), false, false));
        assert!(!should_show_button(None, false, false));
    }
}
