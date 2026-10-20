//! Which packages this Vorn runs: the API version a package was built for
//! against the one this build speaks, and a manifest's `vorn` range.
//!
//! The rule is the WIT's: a minor release only adds, so a package built for
//! `X.Y` runs on a Vorn speaking `X.K` for every `K >= Y`. A newer minor or
//! another major is refused with a message naming both versions, so a person
//! knows whether to update Vorn or the package.

use std::fmt;

use crate::api::{API, PACKAGE};

/// A version of the extension API. A patch changes nothing a package may
/// call, so only `major` and `minor` decide what runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ApiVersion {
    pub const fn new(major: u32, minor: u32) -> ApiVersion {
        ApiVersion {
            major,
            minor,
            patch: 0,
        }
    }

    /// The version this build speaks, from the WIT package.
    pub fn host() -> ApiVersion {
        // The build script wrote PACKAGE from the WIT, which it checked.
        ApiVersion::of(PACKAGE).unwrap_or(ApiVersion::new(0, 0))
    }

    /// Reads `vorn:extension@1.0`, a manifest's `api`, or a component
    /// import's `vorn:extension/host@1.0.0`; `None` for anything else.
    pub fn of(api: &str) -> Option<ApiVersion> {
        let (name, version) = api.rsplit_once('@')?;
        let name = name.split_once('/').map_or(name, |(package, _)| package);
        (name == package_name()).then_some(())?;
        let mut parts = version.split('.').map(number);
        let major = parts.next()??;
        let minor = parts.next()??;
        let patch = parts.next().unwrap_or(Some(0))?;
        parts.next().is_none().then_some(ApiVersion {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}.{}", package_name(), self.major, self.minor)
    }
}

/// `vorn:extension`, the package every API version names.
fn package_name() -> &'static str {
    API.split_once('@').map_or(API, |(name, _)| name)
}

fn number(part: &str) -> Option<u32> {
    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        .then(|| part.parse().ok())
        .flatten()
}

/// Whether a package built for `built` runs on a Vorn speaking `host`, and
/// if not, why, naming both.
pub fn admit(built: ApiVersion, host: ApiVersion) -> Result<(), String> {
    if built.major == host.major && built.minor <= host.minor {
        return Ok(());
    }
    if built > host {
        Err(format!(
            "built for {built}; this Vorn speaks {host}, so update Vorn to run it"
        ))
    } else {
        Err(format!(
            "built for {built}, which this Vorn no longer speaks; it speaks {host}, so rebuild it against {host}"
        ))
    }
}

/// [`admit`] for a manifest's `api` as written.
pub fn admit_api(api: &str, host: ApiVersion) -> Result<(), String> {
    let built = ApiVersion::of(api)
        .ok_or_else(|| format!("`{api}` is not an extension API; this Vorn speaks {host}"))?;
    admit(built, host)
}

/// The API version a component was built against, from the names of its
/// imports, `vorn:extension/host@1.0.0`; `None` when it imports none of them.
pub fn imported<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<ApiVersion> {
    let prefix = format!("{}/", package_name());
    names
        .into_iter()
        .filter(|n| n.starts_with(&prefix))
        .filter_map(ApiVersion::of)
        .max()
}

/// The API label packages carried before 1.0, whose surface 1.0 kept unchanged.
const RELABELLED: &[&str] = &["vorn:extension@0.9", "vorn:extension@0.9.0"];

/// A manifest's text with a pre-1.0 `api` relabelled as this Vorn's, the
/// rest kept byte for byte; `None` when there is nothing to relabel.
pub fn relabel(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    let mut top = true;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        top &= !trimmed.starts_with('[');
        if top && !changed {
            if let Some(next) = relabel_line(line) {
                out.push_str(&next);
                changed = true;
                continue;
            }
        }
        out.push_str(line);
    }
    changed.then_some(out)
}

/// `api = "vorn:extension@0.9"` as `api = "<API>"`, keeping spacing and any comment.
fn relabel_line(line: &str) -> Option<String> {
    let (key, rest) = line.split_once('=')?;
    if key.trim() != "api" {
        return None;
    }
    let value = rest.trim_start();
    let quoted = value.strip_prefix('"')?;
    let end = quoted.find('"')?;
    if !RELABELLED.contains(&&quoted[..end]) {
        return None;
    }
    let lead = &rest[..rest.len() - value.len()];
    Some(format!("{key}={lead}\"{API}\"{}", &quoted[end + 1..]))
}

/// A comparator of a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
    Caret,
    Tilde,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Comparator {
    op: Op,
    version: ApiVersion,
    /// How many of major, minor and patch were written: `^1` is not `^1.0`.
    parts: usize,
}

impl Comparator {
    fn parse(part: &str) -> Result<Comparator, String> {
        let digits = part.trim_start_matches(|c: char| !c.is_ascii_alphanumeric() && c != '*');
        let written = &part[..part.len() - digits.len()];
        let op = match written {
            ">=" => Op::Ge,
            ">" => Op::Gt,
            "<=" => Op::Le,
            "<" => Op::Lt,
            "=" => Op::Eq,
            "^" | "" => Op::Caret,
            "~" => Op::Tilde,
            _ => {
                return Err(format!(
                    "`{part}` starts with `{written}`, which is not a comparator; use >=, >, <=, <, =, ^ or ~"
                ))
            }
        };
        let numbers: Vec<Option<u32>> = digits.trim().split('.').map(number).collect();
        let parts = numbers.len();
        if digits.trim().is_empty() || parts > 3 || numbers.iter().any(Option::is_none) {
            return Err(format!(
                "`{part}` does not name a version like 1.0; an API version is MAJOR.MINOR"
            ));
        }
        let at = |i: usize| numbers.get(i).copied().flatten().unwrap_or(0);
        Ok(Comparator {
            op,
            version: ApiVersion {
                major: at(0),
                minor: at(1),
                patch: at(2),
            },
            parts,
        })
    }

    /// `v` to as many parts as the comparator wrote.
    fn cut(&self, v: ApiVersion) -> (u32, u32, u32) {
        match self.parts {
            1 => (v.major, 0, 0),
            2 => (v.major, v.minor, 0),
            _ => (v.major, v.minor, v.patch),
        }
    }

    fn matches(&self, v: ApiVersion) -> bool {
        let c = self.version;
        match self.op {
            Op::Ge => v >= c,
            // A partial version is its whole family: `<=1` holds 1.2 and `>1` starts at 2.
            Op::Gt => self.cut(v) > self.cut(c),
            Op::Le => self.cut(v) <= self.cut(c),
            Op::Lt => v < c,
            Op::Eq => match self.parts {
                1 => v.major == c.major,
                2 => (v.major, v.minor) == (c.major, c.minor),
                _ => v == c,
            },
            Op::Tilde => {
                v >= c
                    && if self.parts == 1 {
                        v.major == c.major
                    } else {
                        (v.major, v.minor) == (c.major, c.minor)
                    }
            }
            // Below 1.0 a minor is breaking, as in the WIT's own semver.
            Op::Caret => {
                v >= c
                    && if c.major > 0 || self.parts == 1 {
                        v.major == c.major
                    } else {
                        (v.major, v.minor) == (c.major, c.minor)
                    }
            }
        }
    }
}

/// A manifest's `vorn`: the API versions a package runs on, comparators
/// joined by commas, all of which must hold (`>=1.0, <2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    text: String,
    all: Vec<Comparator>,
}

impl Range {
    /// Reads a range; the error names the comparator it could not read.
    pub fn parse(text: &str) -> Result<Range, String> {
        if text.trim().is_empty() {
            return Err("is empty; write the API versions it runs on, like >=1.0".into());
        }
        let all = text
            .split(',')
            .map(str::trim)
            .map(|part| {
                if part.is_empty() {
                    Err(format!("`{text}` has an empty comparator between commas"))
                } else {
                    Comparator::parse(part)
                }
            })
            .collect::<Result<_, _>>()?;
        Ok(Range {
            text: text.trim().to_owned(),
            all,
        })
    }

    pub fn matches(&self, v: ApiVersion) -> bool {
        self.all.iter().all(|c| c.matches(v))
    }

    /// Whether a Vorn speaking `host` is in the range, and if not, why,
    /// naming both.
    pub fn admit(&self, host: ApiVersion) -> Result<(), String> {
        if self.matches(host) {
            return Ok(());
        }
        let newer = self.all.iter().any(|c| {
            matches!(c.op, Op::Ge | Op::Gt | Op::Eq | Op::Caret | Op::Tilde) && c.version > host
        });
        let advice = if newer {
            "update Vorn"
        } else {
            "update the extension"
        };
        Err(format!(
            "runs on {} {}; this Vorn speaks {host}, so {advice} to run it",
            package_name(),
            self.text
        ))
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1_0: ApiVersion = ApiVersion::new(1, 0);
    const V1_2: ApiVersion = ApiVersion::new(1, 2);

    #[test]
    fn this_build_speaks_the_wit_package() {
        assert_eq!(ApiVersion::host(), V1_0);
        assert_eq!(ApiVersion::host().to_string(), API);
    }

    #[test]
    fn reads_a_manifest_api_and_an_import_name() {
        assert_eq!(ApiVersion::of("vorn:extension@1.0"), Some(V1_0));
        assert_eq!(
            ApiVersion::of("vorn:extension/host@1.2.3"),
            Some(ApiVersion {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        for bad in [
            "vorn:extension",
            "vorn:extension@1",
            "vorn:extension@1.x",
            "vorn:extension@1.0.0.0",
            "vorn:connector@1.0",
            "wasi:io/streams@0.2.0",
        ] {
            assert_eq!(ApiVersion::of(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_package_built_for_an_older_minor_or_the_same_version_runs() {
        assert_eq!(admit(V1_0, V1_0), Ok(()));
        assert_eq!(admit(V1_0, V1_2), Ok(()));
        assert_eq!(admit(ApiVersion::new(1, 2), V1_2), Ok(()));
    }

    #[test]
    fn a_package_built_for_a_newer_api_is_refused_naming_both() {
        let err = admit(ApiVersion::new(1, 1), V1_0).unwrap_err();
        assert_eq!(
            err,
            "built for vorn:extension@1.1; this Vorn speaks vorn:extension@1.0, so update Vorn to run it"
        );
        let err = admit(ApiVersion::new(2, 0), V1_2).unwrap_err();
        assert!(err.contains("vorn:extension@2.0") && err.contains("vorn:extension@1.2"));
    }

    #[test]
    fn a_package_built_for_an_older_major_is_refused_naming_both() {
        let err = admit(ApiVersion::new(0, 9), V1_0).unwrap_err();
        assert_eq!(
            err,
            "built for vorn:extension@0.9, which this Vorn no longer speaks; it speaks vorn:extension@1.0, so rebuild it against vorn:extension@1.0"
        );
    }

    #[test]
    fn an_api_that_is_not_one_says_what_this_vorn_speaks() {
        let err = admit_api("vorn:connector@1", V1_0).unwrap_err();
        assert!(err.contains("`vorn:connector@1`") && err.contains("vorn:extension@1.0"));
    }

    #[test]
    fn a_component_is_built_for_the_version_its_imports_name() {
        let imports = [
            "wasi:io/streams@0.2.6",
            "vorn:extension/types@1.0.0",
            "vorn:extension/host@1.0.0",
        ];
        assert_eq!(imported(imports), Some(V1_0));
        assert_eq!(imported(["wasi:cli/environment@0.2.6"]), None);
    }

    #[test]
    fn relabels_only_a_top_level_pre_1_0_api() {
        let old = "id = \"a.b\"\napi  =  \"vorn:extension@0.9\" # kept\n[x]\napi = \"vorn:extension@0.9\"\n";
        assert_eq!(
            relabel(old).unwrap(),
            "id = \"a.b\"\napi  =  \"vorn:extension@1.0\" # kept\n[x]\napi = \"vorn:extension@0.9\"\n"
        );
        assert_eq!(relabel("api = \"vorn:extension@0.9.0\"").unwrap(), "api = \"vorn:extension@1.0\"");
        assert_eq!(relabel("api = \"vorn:extension@1.0\"\n"), None);
        assert_eq!(relabel("api = \"vorn:extension@0.8\"\n"), None);
        assert_eq!(relabel("[a]\napi = \"vorn:extension@0.9\"\n"), None);
        assert_eq!(relabel("apis = \"vorn:extension@0.9\"\n"), None);
    }

    #[test]
    fn ranges_hold_their_comparators() {
        let at = |r: &str, v: ApiVersion| Range::parse(r).unwrap().matches(v);
        assert!(at(">=1.0", V1_0));
        assert!(at(">=0.9", V1_0));
        assert!(!at(">=1.1", V1_0));
        assert!(at(">=1.0, <2", V1_2));
        assert!(!at(">1.0", V1_0));
        assert!(at("<=1.0", V1_0));
        assert!(!at("<1.0", V1_0));
        assert!(at("=1.0", V1_0));
        assert!(!at("=1.0", V1_2));
        assert!(at("=1", V1_2));
        assert!(at("^1.0", V1_2));
        assert!(at("1.0", V1_2));
        assert!(!at("^1.0", ApiVersion::new(2, 0)));
        assert!(!at("^0.9", V1_0));
        assert!(at("~1.0", V1_0));
        assert!(!at("~1.0", V1_2));
        assert!(at("~1", V1_2));
        assert!(at("<=1", V1_2));
        assert!(!at(">1", ApiVersion::new(1, 1)));
        assert!(at(">1", ApiVersion::new(2, 0)));
        assert!(at("<=1.0", ApiVersion { patch: 3, ..V1_0 }));
        assert!(!at(">1.0", ApiVersion { patch: 3, ..V1_0 }));
    }

    #[test]
    fn a_range_that_does_not_read_names_the_comparator() {
        let err = |r: &str| Range::parse(r).unwrap_err();
        assert_eq!(
            err("=>1.0"),
            "`=>1.0` starts with `=>`, which is not a comparator; use >=, >, <=, <, =, ^ or ~"
        );
        assert_eq!(
            err(">=1.x"),
            "`>=1.x` does not name a version like 1.0; an API version is MAJOR.MINOR"
        );
        assert!(err(">=").contains("`>=`"));
        assert!(err("1.0.0.0").contains("`1.0.0.0`"));
        assert!(err(">=1.0,,<2").contains("empty comparator"));
        assert!(err(" ").starts_with("is empty"));
    }

    #[test]
    fn a_range_that_leaves_this_vorn_out_says_which_to_update() {
        let newer = Range::parse(">=1.1").unwrap().admit(V1_0).unwrap_err();
        assert_eq!(
            newer,
            "runs on vorn:extension >=1.1; this Vorn speaks vorn:extension@1.0, so update Vorn to run it"
        );
        let older = Range::parse("<1.0").unwrap().admit(V1_0).unwrap_err();
        assert!(
            older.ends_with("so update the extension to run it"),
            "{older}"
        );
        assert_eq!(Range::parse(">=1.0").unwrap().admit(V1_0), Ok(()));
    }
}
