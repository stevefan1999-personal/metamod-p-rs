#![feature(trivial_bounds)]

/// Raw bindgen output for the selected metamod headers (`fallguys` feature:
/// metamod-svencoop, default: Metamod-P). Everything here is `unsafe` to use.
#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    clippy::all
)]
pub mod sys {
    use const_default::ConstDefault;

    include!(concat!(env!("OUT_DIR"), "/wrapper.rs"));
}
