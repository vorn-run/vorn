//! What the operating system says about this machine: its name and its
//! network addresses, read as Node's `os.hostname()` and
//! `os.networkInterfaces()` read them (both are libuv's).

use std::net::Ipv4Addr;

/// The addresses a browser on another machine could use: IPv4, not
/// loopback, and not link-local (`169.254.x` means DHCP failed), on
/// interfaces that are up, grouped by interface in the order the system
/// lists them.
pub fn lan_addresses() -> Vec<String> {
    let mut by_interface: Vec<(String, Vec<String>)> = Vec::new();
    for found in interfaces() {
        let i = match by_interface.iter().position(|(n, _)| *n == found.name) {
            Some(i) => i,
            None => {
                by_interface.push((found.name.clone(), Vec::new()));
                by_interface.len() - 1
            }
        };
        if let Some(v4) = found.ipv4.filter(|_| !found.internal) {
            if !(v4.octets()[0] == 169 && v4.octets()[1] == 254) {
                by_interface[i].1.push(v4.to_string());
            }
        }
    }
    by_interface.into_iter().flat_map(|(_, a)| a).collect()
}

/// One address of one interface.
struct Found {
    name: String,
    /// `None` for an address of another family, kept for the order.
    ipv4: Option<Ipv4Addr>,
    internal: bool,
}

#[cfg(unix)]
fn interfaces() -> Vec<Found> {
    use std::ffi::CStr;

    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` fills `head` with a list it allocated, freed below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the list, which lives until it is freed.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        let flags = ifa.ifa_flags as libc::c_int;
        if flags & libc::IFF_UP == 0 || flags & libc::IFF_RUNNING == 0 || ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: checked non-null; every address starts with its family.
        let family = libc::c_int::from(unsafe { (*ifa.ifa_addr).sa_family });
        if family != libc::AF_INET && family != libc::AF_INET6 {
            continue;
        }
        // SAFETY: the name is a NUL-terminated string in the list.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let ipv4 = (family == libc::AF_INET).then(|| {
            // SAFETY: an AF_INET address is a `sockaddr_in`.
            let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
            Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr))
        });
        out.push(Found {
            name,
            ipv4,
            internal: flags & libc::IFF_LOOPBACK != 0,
        });
    }
    // SAFETY: `head` came from `getifaddrs` and is freed once.
    unsafe { libc::freeifaddrs(head) };
    out
}

#[cfg(windows)]
fn interfaces() -> Vec<Found> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_PREFIX, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_SOFTWARE_LOOPBACK,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN};

    let flags = GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_INCLUDE_PREFIX;
    let mut size: u32 = 15 * 1024;
    let mut buf: Vec<u64> = Vec::new();
    loop {
        buf.resize((size as usize).div_ceil(8), 0);
        // SAFETY: `buf` holds `size` bytes, aligned for the structures.
        let rc = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                flags,
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if rc == NO_ERROR {
            break;
        }
        if rc != ERROR_BUFFER_OVERFLOW {
            return Vec::new();
        }
    }
    let mut out = Vec::new();
    let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !adapter.is_null() {
        // SAFETY: a node of the list the call wrote into `buf`.
        let a = unsafe { &*adapter };
        adapter = a.Next;
        if a.OperStatus != IfOperStatusUp || a.FirstUnicastAddress.is_null() {
            continue;
        }
        let name = wide_to_string(a.FriendlyName);
        let internal = a.IfType == IF_TYPE_SOFTWARE_LOOPBACK;
        let mut unicast = a.FirstUnicastAddress;
        while !unicast.is_null() {
            // SAFETY: a node of the adapter's unicast list.
            let u = unsafe { &*unicast };
            unicast = u.Next;
            let sa = u.Address.lpSockaddr;
            if sa.is_null() {
                continue;
            }
            // SAFETY: checked non-null; every address starts with its family.
            let family = unsafe { (*sa).sa_family };
            if family != AF_INET && family != AF_INET6 {
                continue;
            }
            let ipv4 = (family == AF_INET).then(|| {
                // SAFETY: an AF_INET address is a `SOCKADDR_IN`.
                let sin = unsafe { &*(sa as *const SOCKADDR_IN) };
                // SAFETY: reading the address as one integer is always valid.
                Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }))
            });
            out.push(Found {
                name: name.clone(),
                ipv4,
                internal,
            });
        }
    }
    out
}

#[cfg(windows)]
fn wide_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    // SAFETY: a NUL-terminated wide string from the system.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` units were just read.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// This machine's name, or empty when the system will not say.
#[cfg(unix)]
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer and its length are passed together.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// This machine's name, or empty when the system will not say.
#[cfg(windows)]
pub fn hostname() -> String {
    use windows_sys::Win32::Networking::WinSock::{GetHostNameW, WSAStartup, WSADATA};

    // Winsock counts its users, so starting it again is harmless.
    // SAFETY: `data` is written by the call.
    let mut data: WSADATA = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    unsafe { WSAStartup(0x0202, &mut data) };
    let mut buf = [0u16; 256];
    // SAFETY: the buffer and its length are passed together.
    if unsafe { GetHostNameW(buf.as_mut_ptr(), buf.len() as i32) } != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_has_a_name_and_no_loopback_lan_address() {
        assert!(!hostname().is_empty());
        for addr in lan_addresses() {
            let ip: Ipv4Addr = addr.parse().unwrap();
            assert!(!ip.is_loopback(), "{addr}");
            assert!(!ip.is_link_local(), "{addr}");
        }
    }
}
