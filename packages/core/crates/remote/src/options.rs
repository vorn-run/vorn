//! The extra ssh options a host may carry, read against an allowlist: `-o
//! Key=Value` for the keys in [`KEYS`], a few switches and `-J`. Anything
//! else is refused by name, as is a user or host name, jump hop or key path
//! that ssh would read as an option. What passes holds no character a POSIX
//! shell treats specially.

use std::fmt;

/// Why a host's login was refused, in words the person can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// The `-o` keys a host may set, lowercase: ssh reads keys in any case.
const KEYS: &[&str] = &[
    "addkeystoagent",
    "addressfamily",
    "bindaddress",
    "bindinterface",
    "canonicaldomains",
    "canonicalizefallbacklocal",
    "canonicalizehostname",
    "canonicalizemaxdots",
    "checkhostip",
    "ciphers",
    "compression",
    "connectionattempts",
    "connecttimeout",
    "controlmaster",
    "controlpath",
    "controlpersist",
    "escapechar",
    "fingerprinthash",
    "forwardagent",
    "forwardx11",
    "forwardx11timeout",
    "forwardx11trusted",
    "gssapiauthentication",
    "gssapidelegatecredentials",
    "hashknownhosts",
    "hostbasedauthentication",
    "hostkeyalgorithms",
    "hostkeyalias",
    "identitiesonly",
    "identityagent",
    "identityfile",
    "ipqos",
    "kbdinteractiveauthentication",
    "kexalgorithms",
    "loglevel",
    "macs",
    "numberofpasswordprompts",
    "passwordauthentication",
    "preferredauthentications",
    "proxyjump",
    "pubkeyacceptedalgorithms",
    "pubkeyauthentication",
    "requesttty",
    "sendenv",
    "serveraliveinterval",
    "serveralivecountmax",
    "stricthostkeychecking",
    "tcpkeepalive",
    "updatehostkeys",
    "userknownhostsfile",
    "verifyhostkeydns",
    "visualhostkey",
];

/// Switches that take no value and change nothing outside the connection.
const SWITCHES: &str = "46ACaqvXxYy";

/// A value is only characters no shell treats specially.
fn plain(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=@+,%~".contains(c))
}

/// A jump host list (`ProxyJump`, `-J`): every hop a `[ssh://][user@]host[:port]`
/// whose user and host ssh reads as names, never as options.
fn jumps(value: &str) -> bool {
    plain(value) && value.split(',').all(hop)
}

/// One jump hop: neither its user nor its host starts with `-` or is missing.
fn hop(hop: &str) -> bool {
    let rest = hop.strip_prefix("ssh://").unwrap_or(hop);
    let (user, host) = match rest.rsplit_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, rest),
    };
    let user_ok = user.is_none_or(|u| !u.is_empty() && !u.starts_with('-'));
    user_ok && !host.is_empty() && !host.starts_with(['-', ':'])
}

/// `-o` with `setting`, `Key=Value`, when the key is allowed and the value plain.
fn setting(setting: &str) -> Result<[String; 2], Refused> {
    let (key, value) = setting.split_once('=').unwrap_or((setting, ""));
    let name = if key.is_empty() { setting } else { key };
    let lower = key.to_ascii_lowercase();
    if !KEYS.contains(&lower.as_str()) {
        return Err(Refused(format!("ssh option {name} is not allowed")));
    }
    let fine = if lower == "proxyjump" {
        jumps(value)
    } else {
        plain(value)
    };
    if !fine {
        return Err(Refused(format!(
            "ssh option {key} has a value that is not allowed"
        )));
    }
    Ok(["-o".to_owned(), format!("{key}={value}")])
}

/// `text`, the options a person typed for a host, as ssh arguments: every
/// one on the allowlist, or the first that is not, named.
pub fn parse(text: &str) -> Result<Vec<String>, Refused> {
    let mut words = text.split_whitespace();
    let mut args = Vec::new();
    while let Some(word) = words.next() {
        match word {
            "-o" => {
                let next = words.next().unwrap_or_default();
                args.extend(setting(next)?);
            }
            "-J" => {
                let next = words.next().unwrap_or_default();
                if !jumps(next) {
                    return Err(Refused(
                        "ssh option -J has a value that is not allowed".into(),
                    ));
                }
                args.extend(["-J".to_owned(), next.to_owned()]);
            }
            _ if word.starts_with("-o") => args.extend(setting(&word[2..])?),
            _ if word.len() > 1
                && word.starts_with('-')
                && word[1..].chars().all(|c| SWITCHES.contains(c)) =>
            {
                args.push(word.to_owned());
            }
            _ => return Err(Refused(format!("ssh option {word} is not allowed"))),
        }
    }
    Ok(args)
}

/// Whether `user` and `hostname` are names ssh reads as such: not empty
/// (the user may be), never starting with `-`, and only plain characters.
pub fn check_destination(user: &str, hostname: &str) -> Result<(), Refused> {
    let name = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.:%+".contains(c))
    };
    if hostname.is_empty() || hostname.starts_with('-') || !name(hostname) {
        return Err(Refused(format!("host name {hostname:?} is not allowed")));
    }
    if user.starts_with('-') || !name(user) {
        return Err(Refused(format!("user name {user:?} is not allowed")));
    }
    Ok(())
}

/// Whether `path`, a key file for `-i`, is one ssh reads as a path: not
/// empty and not starting with `-`.
pub fn check_key_path(path: &str) -> Result<(), Refused> {
    if path.is_empty() || path.starts_with('-') {
        return Err(Refused(format!("ssh key path {path:?} is not allowed")));
    }
    Ok(())
}

/// `port` as the word passed to `-p`: a whole number from 1 to 65535.
pub fn port(port: f64) -> Result<u16, Refused> {
    if port.fract() == 0.0 && (1.0..=65535.0).contains(&port) {
        // In range and whole, so the cast is exact.
        return Ok(port as u16);
    }
    Err(Refused(format!("port {port} is not allowed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(text: &str) -> String {
        parse(text).unwrap_err().to_string()
    }

    #[test]
    fn keeps_allowed_options_in_every_spelling() {
        assert_eq!(
            parse("  -o\tServerAliveInterval=30 -oProxyJump=me@hop:22,hop2 -A -4C -J jump ")
                .unwrap()
                .join(" "),
            "-o ServerAliveInterval=30 -o ProxyJump=me@hop:22,hop2 -A -4C -J jump"
        );
        assert_eq!(parse("").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse("-J ssh://me@hop:2222,me@hop2").unwrap(),
            ["-J", "ssh://me@hop:2222,me@hop2"]
        );
        assert_eq!(
            parse("-o identityfile=~/.ssh/id_ed25519").unwrap(),
            ["-o", "identityfile=~/.ssh/id_ed25519"]
        );
    }

    #[test]
    fn names_the_option_it_refuses() {
        for (text, named) in [
            (
                "-o ProxyCommand=nc",
                "ssh option ProxyCommand is not allowed",
            ),
            ("-oLocalCommand=x", "ssh option LocalCommand is not allowed"),
            (
                "-o KnownHostsCommand=x",
                "ssh option KnownHostsCommand is not allowed",
            ),
            (
                "-o permitlocalcommand=yes",
                "ssh option permitlocalcommand is not allowed",
            ),
            ("-o Match", "ssh option Match is not allowed"),
            ("-o Include=/etc/x", "ssh option Include is not allowed"),
            ("-A -L 80:x:80", "ssh option -L is not allowed"),
            ("-F /tmp/config", "ssh option -F is not allowed"),
            ("-Ao", "ssh option -Ao is not allowed"),
            ("box", "ssh option box is not allowed"),
            ("-o", "ssh option  is not allowed"),
            ("-o =x", "ssh option =x is not allowed"),
        ] {
            assert_eq!(refused(text), named, "{text}");
        }
    }

    #[test]
    fn refuses_values_a_shell_or_ssh_would_read_as_more() {
        for text in [
            "-o ProxyJump=-oProxyCommand=x",
            "-J -oProxyCommand=x",
            "-J hop,-x",
            "-J me@-oProxyCommand=x",
            "-o ProxyJump=ssh://-oProxyCommand=x",
            "-J ssh://me@-x:22",
            "-J @box",
            "-J -me@box",
            "-J box,,hop",
            "-J ssh://:22",
            "-o SendEnv=A;B",
            "-o ControlPath=$(x)",
            "-o LogLevel=",
            "-o HostKeyAlias=`x`",
        ] {
            assert!(
                refused(text).contains("has a value that is not allowed"),
                "{text}"
            );
        }
    }

    #[test]
    fn refuses_a_destination_ssh_would_read_as_an_option() {
        assert!(check_destination("me", "box.example").is_ok());
        assert!(check_destination("", "fe80::1%en0").is_ok());
        assert_eq!(
            check_destination("me", "-oProxyCommand=x")
                .unwrap_err()
                .to_string(),
            "host name \"-oProxyCommand=x\" is not allowed"
        );
        assert_eq!(
            check_destination("-x", "box").unwrap_err().to_string(),
            "user name \"-x\" is not allowed"
        );
        for (user, host) in [("me", ""), ("me", "a b"), ("a;b", "box"), ("me", "box`x`")] {
            assert!(check_destination(user, host).is_err(), "{user}@{host}");
        }
    }

    #[test]
    fn refuses_a_key_path_or_port_ssh_would_misread() {
        assert!(check_key_path("~/.ssh/id_ed25519").is_ok());
        assert!(check_key_path("/k/my key").is_ok());
        assert_eq!(
            check_key_path("-oProxyCommand=x").unwrap_err().to_string(),
            "ssh key path \"-oProxyCommand=x\" is not allowed"
        );
        assert!(check_key_path("").is_err());
        assert_eq!(port(22.0), Ok(22));
        assert_eq!(port(65535.0), Ok(65535));
        for bad in [0.0, -1.0, 22.5, 65536.0, f64::NAN, f64::INFINITY] {
            assert!(port(bad).is_err(), "{bad}");
        }
    }
}
