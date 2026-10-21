//! The program tier of the bench: the same source the component is built
//! from, served on stdio.

fn main() {
    vorn_bench_ext_tiers::program::serve::<vorn_bench_worktrees::Cleanup>();
}
