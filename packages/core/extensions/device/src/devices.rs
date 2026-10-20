//! The simulators a session may take: who holds which, and the picker's
//! order. What `simctl` lists comes from the client (`video.call` `list`);
//! the holders are this extension's own claims.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// One simulator, as `device:list` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub udid: String,
    pub name: String,
    #[serde(default)]
    pub runtime: String,
    #[serde(default)]
    pub booted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// Free, or already this session's.
pub fn is_selectable(d: &Device, session: &str) -> bool {
    d.claimed_by.as_deref().is_none_or(|c| c == session)
}

/// The devices the client listed, each with the session holding it.
pub fn held(mut devices: Vec<Device>, holders: &HashMap<String, String>) -> Vec<Device> {
    for d in &mut devices {
        d.claimed_by = holders.get(&d.udid).cloned();
    }
    devices
}

/// Selectable first, then booted, then by name and runtime;
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn listed_devices_name_their_holder() {
        let holders = HashMap::from([("u1".to_owned(), "s".to_owned())]);
        let all = held(
            vec![
                dev("u1", "iPhone", "iOS 26.2", true, None),
                dev("u2", "iPad", "iOS 26.2", false, Some("stale")),
            ],
            &holders,
        );
        assert_eq!(all[0].claimed_by.as_deref(), Some("s"));
        assert_eq!(all[1].claimed_by, None, "only a claim holds a device");
        assert!(is_selectable(&all[0], "s") && !is_selectable(&all[0], "t"));
    }
}
