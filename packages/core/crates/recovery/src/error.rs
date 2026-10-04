//! The crate's one error type.
//!
//! A real engine plugged in through [`crate::Engine`] maps its own errors to
//! [`Error::Engine`] with a message: the harness reports them, it never
//! branches on them.

use std::fmt;

use vorn_term_proto::Cursor;

use crate::compare::Mismatch;
use crate::log::Digest;

#[derive(Debug)]
pub enum Error {
    Screen(vorn_screen::Error),
    Ghostty(libghostty_vt::Error),
    Io(std::io::Error),
    Codec(postcard::Error),
    /// A log or transcript that is not numbered as a record log must be.
    Log(String),
    /// A cursor that is not a record boundary of the log: what a recovery
    /// that lost or invented bytes looks like from outside.
    Cursor {
        cursor: Cursor,
        end: Cursor,
    },
    /// A target resumed after records it was never given.
    ResumedAhead {
        resumed: Cursor,
        delivered: Cursor,
    },
    /// A record the harness does not model, such as a Gap (resetting the
    /// terminal across epochs is out of scope).
    Unsupported(&'static str),
    /// The engine under test failed; its own words.
    Engine(String),
    /// The subject process broke the protocol or died on its own.
    Subject(String),
    /// A record was delivered to a target that was killed and not recovered.
    Dead,
    /// The two terminals of a differential run are not equivalent.
    Mismatch(Mismatch),
    /// The recovered run was not built from the log's bytes, once each.
    Stream {
        expected: Digest,
        got: Digest,
    },
}

impl Error {
    /// The mismatch, when the error is one.
    pub fn mismatch(&self) -> Option<&Mismatch> {
        match self {
            Error::Mismatch(m) => Some(m),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Screen(e) => write!(f, "screen: {e}"),
            Error::Ghostty(e) => write!(f, "libghostty-vt: {e}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Codec(e) => write!(f, "encoding: {e}"),
            Error::Log(m) => write!(f, "malformed log: {m}"),
            Error::Cursor { cursor, end } => {
                write!(
                    f,
                    "cursor {cursor:?} is not a record boundary of a log ending at {end:?}"
                )
            }
            Error::ResumedAhead { resumed, delivered } => {
                write!(
                    f,
                    "resumed at {resumed:?}, after the last record delivered ({delivered:?})"
                )
            }
            Error::Unsupported(what) => write!(f, "not modelled by the harness: {what}"),
            Error::Engine(m) => write!(f, "engine: {m}"),
            Error::Subject(m) => write!(f, "subject process: {m}"),
            Error::Dead => write!(f, "record delivered to a killed target"),
            Error::Mismatch(m) => write!(f, "{m}"),
            Error::Stream { expected, got } => {
                write!(f, "recovered stream {got:?} is not the log's {expected:?}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<vorn_screen::Error> for Error {
    fn from(e: vorn_screen::Error) -> Self {
        Error::Screen(e)
    }
}

impl From<libghostty_vt::Error> for Error {
    fn from(e: libghostty_vt::Error) -> Self {
        Error::Ghostty(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<postcard::Error> for Error {
    fn from(e: postcard::Error) -> Self {
        Error::Codec(e)
    }
}

impl From<Mismatch> for Error {
    fn from(m: Mismatch) -> Self {
        Error::Mismatch(m)
    }
}
