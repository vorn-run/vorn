//! The vornd client: finding it ([`endpoint`]), calling it ([`rpc`]) and
//! the models the screen reads, kept current from its notifications
//! ([`store`]).

pub mod endpoint;
pub mod link;
pub mod report;
pub mod rpc;
pub mod store;
pub mod sync;

pub use endpoint::{data_dir, Endpoint, FindError};
pub use rpc::{Event, Pending, Rpc, RpcError};
pub use store::{Change, Session, Store};

/// Standard base64 with padding, for bytes vornd reads as text (a
/// terminal's input, a simulator's screenshot).
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_pads_like_btoa() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }
}
