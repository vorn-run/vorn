//! Why a phase stopped: each one becomes a limit in the results, since
//! where the system gives out is what a scale bench is for.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// A frame that did not decode.
    Protocol(String),
    /// Waited this long for this.
    Timeout(String),
    /// The other side closed this connection.
    Closed(&'static str),
    /// The other side said no.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Protocol(e) => write!(f, "bad frame: {e}"),
            Error::Timeout(what) => write!(f, "timed out waiting for {what}"),
            Error::Closed(conn) => write!(f, "{conn} closed the connection"),
            Error::Failed(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}
