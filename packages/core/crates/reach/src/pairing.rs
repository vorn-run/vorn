//! Pairing a phone by showing it a code.
//!
//! A device token never expires, so a QR code carrying one would be a
//! permanent key on a screen. What the screen shows instead is a code that
//! lives five minutes and works once, which the phone trades for a token only
//! after a person approves the exchange on this machine.
//!
//! Held in memory, never stored: a code that survived a restart would be one
//! nobody is watching. Times are milliseconds since the epoch, passed in, so
//! every rule here can be tested without waiting.

use serde_json::{json, Value};

/// How long a code is worth showing.
pub const CODE_TTL_MS: u64 = 5 * 60_000;

/// How long an approval stays collectable.
pub const APPROVAL_TTL_MS: u64 = 5 * 60_000;

/// Wrong codes allowed before the code is abandoned.
pub const MAX_ATTEMPTS: u32 = 10;

/// Crockford's alphabet: nothing reads as a digit, no accidental words.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_LENGTH: usize = 8;

#[derive(Debug)]
struct ActiveCode {
    code: String,
    expires_at: u64,
    attempts: u32,
    /// Set by the first successful redeem: a code is good for one exchange.
    spent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pending,
    Approved,
    Denied,
    Collected,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Approved => "approved",
            Status::Denied => "denied",
            Status::Collected => "collected",
        }
    }
}

#[derive(Debug)]
struct Request {
    id: String,
    device_name: String,
    address: String,
    asked_at: u64,
    status: Status,
    decided_at: Option<u64>,
    /// The code it came from: a late collect must not retire a newer code.
    code: String,
}

impl Request {
    fn to_json(&self) -> Value {
        json!({
            "requestId": self.id,
            "deviceName": self.device_name,
            "address": self.address,
            "askedAt": self.asked_at,
            "status": self.status.name(),
        })
    }
}

/// Why a code was not taken. Each reads as its name on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Unknown,
    Expired,
    Spent,
    Throttled,
}

impl Refused {
    pub fn name(self) -> &'static str {
        match self {
            Refused::Unknown => "unknown",
            Refused::Expired => "expired",
            Refused::Spent => "spent",
            Refused::Throttled => "throttled",
        }
    }
}

/// What came of a request, asked by the phone.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    Pending,
    Denied,
    Expired,
    /// Approved and not yet collected: mint a token named `device_name`,
    /// then call [`Pairing::collected`].
    Approved {
        device_name: String,
    },
}

/// The pairing state of one server.
#[derive(Debug, Default)]
pub struct Pairing {
    active: Option<ActiveCode>,
    /// In the order they were asked.
    requests: Vec<Request>,
}

/// Forty random bits as eight characters of the alphabet, five bits each,
/// so every character is equally likely.
fn generate_code() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 5];
    getrandom::fill(&mut bytes)?;
    let bits = bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
    Ok((0..CODE_LENGTH)
        .map(|i| {
            let shift = (CODE_LENGTH - 1 - i) * 5;
            ALPHABET[((bits >> shift) & 31) as usize] as char
        })
        .collect())
}

/// `ABCD-EFGH`, for reading aloud or typing.
pub fn format_code(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

/// What a person typed, as a code: case, spacing and dashes forgiven.
fn normalise_code(raw: &Value) -> String {
    raw.as_str()
        .map(|s| {
            s.to_uppercase()
                .chars()
                .filter(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
                .collect()
        })
        .unwrap_or_default()
}

/// JavaScript's `trim`: Unicode white space and the byte order mark.
fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// The first `n` UTF-16 units of `s`, never splitting a character.
fn utf16_prefix(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

impl Pairing {
    pub fn new() -> Pairing {
        Pairing::default()
    }

    /// Drops what can no longer be acted on: a request that waited too
    /// long, or was decided too long ago. Left in place, an approval prompt
    /// still on screen would stay answerable.
    fn prune(&mut self, now: u64) {
        self.requests.retain(|r| {
            let waited_too_long =
                r.status == Status::Pending && now.saturating_sub(r.asked_at) >= CODE_TTL_MS;
            let decided_too_long_ago = r
                .decided_at
                .is_some_and(|d| now.saturating_sub(d) >= APPROVAL_TTL_MS);
            !(waited_too_long || decided_too_long_ago)
        });
    }

    fn expired(&self, now: u64) -> bool {
        self.active.as_ref().is_some_and(|a| now >= a.expires_at)
    }

    /// Begins pairing with a new code, abandoning any code showing. Answers
    /// `{code, expiresAt}`.
    pub fn start(&mut self, now: u64) -> Result<Value, getrandom::Error> {
        let code = generate_code()?;
        let expires_at = now + CODE_TTL_MS;
        let shown = format_code(&code);
        self.active = Some(ActiveCode {
            code,
            expires_at,
            attempts: 0,
            spent: false,
        });
        Ok(json!({ "code": shown, "expiresAt": expires_at }))
    }

    /// Stops showing the code; requests still pending go with it.
    pub fn cancel(&mut self) {
        self.active = None;
        self.requests.retain(|r| r.status != Status::Pending);
    }

    /// The phone offers `code`. A match records a request for a person to
    /// decide and hands nothing over. A wrong code and no code at all answer
    /// alike, so the reply never tells whether pairing is open.
    pub fn redeem(
        &mut self,
        code: &Value,
        device_name: &Value,
        address: &str,
        now: u64,
    ) -> Result<String, Refused> {
        self.prune(now);
        let expired = self.expired(now);
        let Some(active) = self.active.as_mut() else {
            return Err(Refused::Unknown);
        };
        if expired {
            return Err(Refused::Expired);
        }
        if active.spent {
            return Err(Refused::Spent);
        }
        if active.attempts >= MAX_ATTEMPTS {
            return Err(Refused::Throttled);
        }
        let offered = normalise_code(code);
        if !crate::token::constant_time_eq(offered.as_bytes(), active.code.as_bytes()) {
            active.attempts += 1;
            return Err(if active.attempts >= MAX_ATTEMPTS {
                Refused::Throttled
            } else {
                Refused::Unknown
            });
        }
        active.spent = true;
        let code = active.code.clone();
        let name = device_name
            .as_str()
            .map(js_trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("Phone");
        let id = uuid::Uuid::new_v4().to_string();
        self.requests.push(Request {
            id: id.clone(),
            device_name: utf16_prefix(name, 64).to_owned(),
            address: address.to_owned(),
            asked_at: now,
            status: Status::Pending,
            decided_at: None,
            code,
        });
        Ok(id)
    }

    /// Every request waiting on a person, as the desktop lists them.
    pub fn pending(&mut self, now: u64) -> Vec<Value> {
        self.prune(now);
        self.requests
            .iter()
            .filter(|r| r.status == Status::Pending && now.saturating_sub(r.asked_at) < CODE_TTL_MS)
            .map(Request::to_json)
            .collect()
    }

    /// The request `id` as the desktop is told of it, while it is pending.
    pub fn pending_one(&mut self, id: &str, now: u64) -> Option<Value> {
        self.pending(now)
            .into_iter()
            .find(|r| r["requestId"].as_str() == Some(id))
    }

    fn decide(&mut self, id: &Value, status: Status, now: u64) -> bool {
        self.prune(now);
        let Some(id) = id.as_str() else {
            return false;
        };
        match self
            .requests
            .iter_mut()
            .find(|r| r.id == id && r.status == Status::Pending)
        {
            Some(r) => {
                r.status = status;
                r.decided_at = Some(now);
                true
            }
            None => false,
        }
    }

    pub fn approve(&mut self, id: &Value, now: u64) -> bool {
        self.decide(id, Status::Approved, now)
    }

    pub fn deny(&mut self, id: &Value, now: u64) -> bool {
        self.decide(id, Status::Denied, now)
    }

    /// The phone asks what came of request `id`.
    pub fn poll(&mut self, id: &Value, now: u64) -> Poll {
        self.prune(now);
        let Some(i) = id
            .as_str()
            .and_then(|id| self.requests.iter().position(|r| r.id == id))
        else {
            return Poll::Expired;
        };
        let r = &self.requests[i];
        match r.status {
            Status::Denied => Poll::Denied,
            Status::Collected => Poll::Expired,
            Status::Pending if now.saturating_sub(r.asked_at) >= CODE_TTL_MS => {
                self.requests.remove(i);
                Poll::Expired
            }
            Status::Pending => Poll::Pending,
            Status::Approved
                if r.decided_at
                    .is_some_and(|d| now.saturating_sub(d) >= APPROVAL_TTL_MS) =>
            {
                self.requests.remove(i);
                Poll::Expired
            }
            Status::Approved => Poll::Approved {
                device_name: r.device_name.clone(),
            },
        }
    }

    /// The token for approved request `id` was minted and handed over: the
    /// request is spent, and so is its code, if it is still the one showing.
    pub fn collected(&mut self, id: &str) {
        let Some(r) = self.requests.iter_mut().find(|r| r.id == id) else {
            return;
        };
        r.status = Status::Collected;
        if self.active.as_ref().is_some_and(|a| a.code == r.code) {
            self.active = None;
        }
    }

    /// The code showing, unformatted; for tests.
    #[cfg(test)]
    fn code(&self) -> Option<String> {
        self.active.as_ref().map(|a| a.code.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    fn started() -> (Pairing, String) {
        let mut p = Pairing::new();
        let shown = p.start(T0).unwrap();
        let code = p.code().unwrap();
        assert_eq!(shown["code"], format_code(&code));
        assert_eq!(shown["expiresAt"], T0 + CODE_TTL_MS);
        (p, code)
    }

    #[test]
    fn codes_are_eight_characters_of_the_alphabet() {
        for _ in 0..200 {
            let code = generate_code().unwrap();
            assert_eq!(code.len(), 8);
            assert!(code.bytes().all(|b| ALPHABET.contains(&b)), "{code}");
        }
    }

    #[test]
    fn a_code_forgives_case_spacing_and_dashes_and_works_once() {
        let (mut p, code) = started();
        let typed = format!(" {}-{} ", code[..4].to_lowercase(), &code[4..]);
        let id = p
            .redeem(&json!(typed), &json!("  Ana's phone "), "10.0.0.2", T0 + 1)
            .unwrap();
        assert_eq!(
            p.pending(T0 + 2),
            vec![json!({
                "requestId": id, "deviceName": "Ana's phone", "address": "10.0.0.2",
                "askedAt": T0 + 1, "status": "pending"
            })]
        );
        assert_eq!(
            p.redeem(&json!(code), &json!(null), "x", T0 + 3),
            Err(Refused::Spent)
        );
    }

    #[test]
    fn nothing_open_and_a_wrong_code_answer_alike_until_throttled() {
        let mut p = Pairing::new();
        assert_eq!(
            p.redeem(&json!("AAAA"), &json!(null), "x", T0),
            Err(Refused::Unknown)
        );
        let (mut p, code) = started();
        for _ in 0..MAX_ATTEMPTS - 1 {
            assert_eq!(
                p.redeem(&json!("WRONG"), &json!(null), "x", T0),
                Err(Refused::Unknown)
            );
        }
        assert_eq!(
            p.redeem(&json!("WRONG"), &json!(null), "x", T0),
            Err(Refused::Throttled)
        );
        assert_eq!(
            p.redeem(&json!(code), &json!(null), "x", T0),
            Err(Refused::Throttled)
        );
    }

    #[test]
    fn a_code_dies_after_five_minutes() {
        let (mut p, code) = started();
        assert_eq!(
            p.redeem(&json!(code), &json!(null), "x", T0 + CODE_TTL_MS),
            Err(Refused::Expired)
        );
    }

    #[test]
    fn approval_is_collected_once_and_retires_its_own_code_only() {
        let (mut p, code) = started();
        let id = p.redeem(&json!(code), &json!(42), "x", T0).unwrap();
        assert_eq!(p.poll(&json!(id), T0 + 1), Poll::Pending);
        assert!(p.approve(&json!(id), T0 + 2));
        assert!(!p.approve(&json!(id), T0 + 2));
        assert!(!p.deny(&json!(id), T0 + 2));
        // A new code is showing by the time the phone collects.
        p.start(T0 + 3).unwrap();
        let newer = p.code().unwrap();
        assert_eq!(
            p.poll(&json!(id), T0 + 4),
            Poll::Approved {
                device_name: "Phone".into()
            }
        );
        p.collected(&id);
        assert_eq!(p.code(), Some(newer));
        assert_eq!(p.poll(&json!(id), T0 + 5), Poll::Expired);
    }

    #[test]
    fn collecting_retires_the_code_that_made_the_request() {
        let (mut p, code) = started();
        let id = p.redeem(&json!(code), &json!(""), "x", T0).unwrap();
        p.approve(&json!(id), T0);
        p.collected(&id);
        assert_eq!(p.code(), None);
    }

    #[test]
    fn denied_and_stale_requests() {
        let (mut p, code) = started();
        let id = p.redeem(&json!(code), &json!("a"), "x", T0).unwrap();
        assert!(p.deny(&json!(id), T0 + 1));
        assert_eq!(p.poll(&json!(id), T0 + 2), Poll::Denied);
        // Decided too long ago: gone.
        assert_eq!(p.poll(&json!(id), T0 + 1 + APPROVAL_TTL_MS), Poll::Expired);
        assert_eq!(p.poll(&json!(7), T0), Poll::Expired);
        assert!(!p.approve(&json!(null), T0));

        let (mut p, code) = started();
        let id = p.redeem(&json!(code), &json!("a"), "x", T0).unwrap();
        assert!(p.pending(T0 + CODE_TTL_MS).is_empty());
        assert!(!p.approve(&json!(id), T0 + CODE_TTL_MS));
    }

    #[test]
    fn cancelling_drops_the_code_and_pending_requests() {
        let (mut p, code) = started();
        let id = p.redeem(&json!(code), &json!("a"), "x", T0).unwrap();
        p.cancel();
        assert!(p.pending(T0).is_empty());
        assert_eq!(p.poll(&json!(id), T0), Poll::Expired);
        assert_eq!(p.code(), None);
    }

    #[test]
    fn names_are_cut_to_sixty_four_utf16_units() {
        assert_eq!(utf16_prefix(&"a".repeat(70), 64).len(), 64);
        let emoji = "😀".repeat(40);
        assert_eq!(utf16_prefix(&emoji, 64).chars().count(), 32);
        assert_eq!(utf16_prefix(&format!("a{emoji}"), 64).chars().count(), 32);
    }
}
