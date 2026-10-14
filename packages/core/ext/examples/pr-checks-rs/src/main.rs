//! The native transport: `cargo run` speaks the same calls over stdio.

fn main() {
    vorn_extension::serve::<pr_checks::PrChecks>();
}
