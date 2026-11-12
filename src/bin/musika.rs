#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test binaries inherit the library test policy"
    )
)]

fn main() {
    musika::entry::run();
}
