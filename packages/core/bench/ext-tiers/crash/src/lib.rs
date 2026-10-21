//! An extension that misbehaves when asked, so the extension-tiers bench can
//! see what each tier does about it: `crash` panics, `spin` never returns,
//! `grow` holds `mb` megabytes, and `echo` answers its args, which is how a
//! test sees the extension, or its neighbour, still answering.

use vorn_extension_sdk::{json, Ctx, Error, Extension, Host, Init, Value};

pub struct Crash;

impl Extension for Crash {
    fn init(_: &Init, _: &mut dyn Host) -> Result<Self, Error> {
        Ok(Crash)
    }

    fn handle(&mut self, _: &mut dyn Host, method: &str, args: Value, _: &Ctx) -> Result<Value, Error> {
        match method {
            "echo" => Ok(args),
            "crash" => panic!("asked to crash"),
            "spin" => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "grow" => {
                let mb = args["mb"].as_u64().unwrap_or(1);
                let held = vec![1u8; usize::try_from(mb << 20).unwrap_or(usize::MAX)];
                Ok(json!(held.iter().map(|b| u64::from(*b)).sum::<u64>()))
            }
            _ => Err(Error::unknown(method)),
        }
    }
}

vorn_extension_sdk::export!(Crash);
