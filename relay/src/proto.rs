//! Wire vocabulary shared by server and client. Handshake and control messages are short
//! ASCII strings; data payloads are opaque bytes the relay never looks into.

use std::fmt;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 27777;
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Target byte of a host frame that addresses every client.
pub const BROADCAST: u8 = 0;
/// Source byte of a relay -> host control frame.
pub const CONTROL: u8 = 0xFF;
/// Client ids run `1..=MAX_CLIENT_ID`.
pub const MAX_CLIENT_ID: u8 = 7;

const MAGIC: &str = "RRLY1 ";

/// The first frame a connection sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handshake {
    Host,
    Join(String),
}

impl Handshake {
    pub fn parse(payload: &[u8]) -> Option<Self> {
        let rest = std::str::from_utf8(payload).ok()?.strip_prefix(MAGIC)?;
        match rest {
            "HOST" => Some(Self::Host),
            _ => rest.strip_prefix("JOIN ").map(|code| Self::Join(code.to_owned())),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Host => format!("{MAGIC}HOST"),
            Self::Join(code) => format!("{MAGIC}JOIN {code}"),
        }
        .into_bytes()
    }
}

/// Why the relay turned a handshake down (`ERR <reason>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Room full on join; room limit reached on host.
    Full,
    NoRoom,
    RateLimited,
    BadRequest,
}

impl Refusal {
    const ALL: [Self; 4] = [Self::Full, Self::NoRoom, Self::RateLimited, Self::BadRequest];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::NoRoom => "no_room",
            Self::RateLimited => "rate_limited",
            Self::BadRequest => "bad_request",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn ok_reply(value: &str) -> Vec<u8> {
    format!("OK {value}").into_bytes()
}

pub fn err_reply(refusal: Refusal) -> Vec<u8> {
    format!("ERR {refusal}").into_bytes()
}

/// `OK <value>` -> `Ok(value)`, `ERR <reason>` -> `Err(reason)`, anything else -> `None`.
pub fn parse_reply(payload: &[u8]) -> Option<Result<&str, Refusal>> {
    let text = std::str::from_utf8(payload).ok()?;
    if let Some(value) = text.strip_prefix("OK ") {
        return Some(Ok(value));
    }
    let reason = text.strip_prefix("ERR ")?;
    Refusal::ALL.into_iter().find(|r| r.as_str() == reason).map(Err)
}

/// Relay -> host notices, sent after the [`CONTROL`] byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Joined(u8),
    Left(u8),
}

impl Control {
    pub fn encode(self) -> Vec<u8> {
        match self {
            Self::Joined(id) => format!("JOINED {id}"),
            Self::Left(id) => format!("LEFT {id}"),
        }
        .into_bytes()
    }

    pub fn parse(text: &[u8]) -> Option<Self> {
        let (word, id) = std::str::from_utf8(text).ok()?.split_once(' ')?;
        let id = id.parse().ok()?;
        match word {
            "JOINED" => Some(Self::Joined(id)),
            "LEFT" => Some(Self::Left(id)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshakes() {
        assert_eq!(Handshake::parse(b"RRLY1 HOST"), Some(Handshake::Host));
        assert_eq!(Handshake::parse(b"RRLY1 JOIN AB23CD"), Some(Handshake::Join("AB23CD".into())));
        for bad in [&b"RRLY1 HOST "[..], b"RRLY2 HOST", b"HOST", b"", b"RRLY1 JOIN", &[0xC3, 0x28]]
        {
            assert_eq!(Handshake::parse(bad), None, "{bad:?}");
        }
        let join = Handshake::Join("XYZ234".into());
        assert_eq!(Handshake::parse(&join.encode()), Some(join));
    }

    #[test]
    fn replies() {
        assert_eq!(parse_reply(b"OK 3"), Some(Ok("3")));
        for r in Refusal::ALL {
            assert_eq!(parse_reply(&err_reply(r)), Some(Err(r)));
        }
        assert_eq!(parse_reply(b"ERR nope"), None);
        assert_eq!(parse_reply(b"KO"), None);
    }

    #[test]
    fn control_round_trip() {
        for c in [Control::Joined(1), Control::Left(7)] {
            assert_eq!(Control::parse(&c.encode()), Some(c));
        }
        assert_eq!(Control::parse(b"JOINED x"), None);
        assert_eq!(Control::parse(b"KICKED 1"), None);
    }
}
