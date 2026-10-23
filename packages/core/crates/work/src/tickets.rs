//! Tickets: how an artifact's page or a gate's review page is opened by
//! address without the address carrying the page's lasting token.
//!
//! A page's token stays with vornd and the clients it talks to. An address
//! handed out carries a ticket instead (`?ticket=`): random, good for one
//! load, for [`TTL_MS`]. The ticket names the page it opens, so one minted
//! for one page opens no other.

use std::collections::HashMap;

/// How long a ticket opens its page.
pub const TTL_MS: i64 = 5 * 60_000;

/// Tickets held at once; the oldest go first past it.
pub const MAX_HELD: usize = 512;

/// The query key an address carries its ticket under.
pub const KEY: &str = "ticket";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    path: String,
    token: String,
    expires_at: i64,
}

/// The tickets minted and not yet used.
#[derive(Debug, Default)]
pub struct Tickets {
    held: HashMap<String, Held>,
}

impl Tickets {
    /// A ticket for the page at `path` (`/artifact/<id>/<version>` or
    /// `/gate-view/<run>/<node>`, no query), which `token` opens.
    pub fn issue(&mut self, path: &str, token: &str, now_ms: i64) -> String {
        self.held.retain(|_, h| h.expires_at > now_ms);
        if self.held.len() >= MAX_HELD {
            if let Some(oldest) = self
                .held
                .iter()
                .min_by_key(|(_, h)| h.expires_at)
                .map(|(t, _)| t.clone())
            {
                self.held.remove(&oldest);
            }
        }
        let ticket = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.held.insert(
            ticket.clone(),
            Held {
                path: path.to_owned(),
                token: token.to_owned(),
                expires_at: now_ms + TTL_MS,
            },
        );
        ticket
    }

    /// Uses `ticket` for the page at `path`: the token that opens it, once,
    /// while the ticket is fresh and was minted for that page. A ticket is
    /// spent whether or not it opened anything.
    pub fn redeem(&mut self, ticket: &str, path: &str, now_ms: i64) -> Option<String> {
        let held = self.held.remove(ticket)?;
        (held.path == path && now_ms < held.expires_at).then_some(held.token)
    }
}

/// `path` with its `?t=<token>` swapped for a fresh ticket from `tickets`;
/// a path with no `t` comes back as it was.
pub fn swap_token(tickets: &mut Tickets, path: &str, now_ms: i64) -> String {
    let Some((page, query)) = path.split_once('?') else {
        return path.to_owned();
    };
    let mut token = None;
    let rest: Vec<&str> = query
        .split('&')
        .filter(|kv| match kv.strip_prefix("t=") {
            Some(t) => {
                token = Some(t);
                false
            }
            None => !kv.is_empty(),
        })
        .collect();
    let Some(token) = token.map(percent_decode) else {
        return path.to_owned();
    };
    let ticket = tickets.issue(&decoded_path(page), &token, now_ms);
    let mut out = format!("{page}?{KEY}={ticket}");
    for kv in rest {
        out.push('&');
        out.push_str(kv);
    }
    out
}

/// A page path as a request names it once its segments are decoded.
pub fn decoded_path(page: &str) -> String {
    page.split('/')
        .map(percent_decode)
        .collect::<Vec<_>>()
        .join("/")
}

/// `%XX` escapes decoded, as the route decodes a path's segments.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_opens_its_own_page_once_while_fresh() {
        let mut t = Tickets::default();
        let ticket = t.issue("/artifact/a/1", "tok", 0);
        assert_eq!(
            t.redeem(&ticket, "/artifact/a/1", 10).as_deref(),
            Some("tok")
        );
        assert_eq!(t.redeem(&ticket, "/artifact/a/1", 10), None);

        let other = t.issue("/artifact/a/1", "tok", 0);
        assert_eq!(t.redeem(&other, "/artifact/b/1", 10), None);
        // Spent by the wrong page as well.
        assert_eq!(t.redeem(&other, "/artifact/a/1", 10), None);

        let late = t.issue("/gate-view/r/n", "v", 0);
        assert_eq!(t.redeem(&late, "/gate-view/r/n", TTL_MS), None);
        assert_eq!(t.redeem("made-up", "/gate-view/r/n", 0), None);
    }

    #[test]
    fn holds_a_bounded_number() {
        let mut t = Tickets::default();
        for i in 0..(MAX_HELD + 20) {
            t.issue("/artifact/a/1", "tok", i64::try_from(i).unwrap());
        }
        assert!(t.held.len() <= MAX_HELD);
    }

    #[test]
    fn swaps_the_token_in_an_address_for_a_ticket() {
        let mut t = Tickets::default();
        let swapped = swap_token(&mut t, "/artifact/a%20b/2?t=s3cret&x=1", 0);
        assert!(!swapped.contains("s3cret"), "{swapped}");
        let ticket = swapped
            .split_once("?ticket=")
            .and_then(|(_, q)| q.split('&').next())
            .unwrap()
            .to_owned();
        assert!(swapped.ends_with("&x=1"));
        assert_eq!(
            t.redeem(&ticket, "/artifact/a b/2", 1).as_deref(),
            Some("s3cret")
        );
        assert_eq!(swap_token(&mut t, "/artifact/a/2", 0), "/artifact/a/2");
    }
}
