//! Which address vornd as the server listens on.
//!
//! A port that moves hands a browser a new origin, and its stored token stays
//! behind at the old one, so the port is kept: one named on the command line
//! wins, else the one this install settled on, else [`DEFAULT_PORT`]. Another
//! is taken only when something else holds the wanted one. It is bound on
//! every interface when Network Access is on, else on loopback.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tokio::net::TcpListener;

/// The port a Vorn server takes when it has no reason to take another.
pub const DEFAULT_PORT: u16 = 50091;

/// The port to ask for.
pub fn wanted(explicit: Option<u16>, remembered: Option<u16>) -> u16 {
    explicit.or(remembered).unwrap_or(DEFAULT_PORT)
}

/// Whether the port bound is worth remembering: never one named on the
/// command line; always one bound as asked; one taken instead of the wanted
/// one only when nothing was remembered, so a second Vorn on one data
/// directory does not move the first.
pub fn remember(explicit: Option<u16>, remembered: Option<u16>, fell_back: bool) -> bool {
    explicit.is_none() && (!fell_back || remembered.is_none())
}

/// Where to listen: every interface with Network Access on, else loopback.
pub fn host(network_access: bool) -> IpAddr {
    if network_access {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    } else {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }
}

/// Binds `host:wanted`, or any port on `host` when that one is taken.
/// Answers the listener and whether it fell back.
pub async fn bind(host: IpAddr, wanted: u16) -> std::io::Result<(TcpListener, bool)> {
    match TcpListener::bind(SocketAddr::new(host, wanted)).await {
        Ok(listener) => Ok((listener, false)),
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse && wanted != 0 => {
            let listener = TcpListener::bind(SocketAddr::new(host, 0)).await?;
            Ok((listener, true))
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_port_this_install_settled_on() {
        assert_eq!(wanted(Some(7), Some(8)), 7);
        assert_eq!(wanted(None, Some(8)), 8);
        assert_eq!(wanted(None, None), DEFAULT_PORT);
        assert_eq!(wanted(Some(0), Some(8)), 0);
    }

    #[test]
    fn remembers_only_what_should_stay() {
        assert!(!remember(Some(7), None, false));
        assert!(remember(None, Some(8), false));
        assert!(!remember(None, Some(8), true));
        assert!(remember(None, None, true));
    }

    #[test]
    fn binds_wide_only_with_network_access() {
        assert!(host(true).is_unspecified());
        assert!(host(false).is_loopback());
    }

    #[tokio::test]
    async fn takes_another_port_when_the_wanted_one_is_held() {
        let (held, fell) = bind(host(false), 0).await.unwrap();
        assert!(!fell);
        let port = held.local_addr().unwrap().port();
        let (other, fell) = bind(host(false), port).await.unwrap();
        assert!(fell);
        assert_ne!(other.local_addr().unwrap().port(), port);
    }
}
