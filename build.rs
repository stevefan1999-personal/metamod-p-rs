use std::{env, path::PathBuf};

use bindgen::{
    callbacks::{DeriveInfo, TypeKind},
    CargoCallbacks,
};
use miette::{IntoDiagnostic, WrapErr};

#[derive(Debug)]
struct ConstDefaultCallbacks;

macro_rules! p {
    ($($tokens: tt)*) => {
        println!("cargo:warning={}", format!($($tokens)*))
    }
}

impl bindgen::callbacks::ParseCallbacks for ConstDefaultCallbacks {
    fn add_derives(&self, info: &DeriveInfo<'_>) -> Vec<String> {
        if info.kind == TypeKind::Struct {
            vec!["ConstDefault".to_string()]
        } else {
            vec![]
        }
    }
}

fn do_bindgen(header: &str, file: &str) -> miette::Result<()> {
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let bindings = bindgen::Builder::default()
        .header(header)
        .clang_args(&[
            "-I",
            "metamod-p",
            "-I",
            "metamod-p/hlsdk",
            "-I",
            "metamod-p/metamod",
            "-I",
            "metamod-p/hlsdk/common",
            "-I",
            "metamod-p/hlsdk/dlls",
            "-I",
            "metamod-p/hlsdk/engine",
            "-I",
            "metamod-p/hlsdk/pm_shared",
            "-x",
            "c++",
        ])
        .allowlist_file(".*?metamod-p.*")
        .vtable_generation(true)
        .ctypes_prefix("::std::ffi")
        .derive_copy(true)
        .derive_hash(true)
        .generate_cstr(true)
        .array_pointers_in_arguments(true)
        .sort_semantically(true)
        .merge_extern_blocks(true)
        .layout_tests(false)
        .newtype_enum(".*")
        .parse_callbacks(Box::new(ConstDefaultCallbacks))
        .parse_callbacks(Box::new(CargoCallbacks::new()))
        .generate()
        .into_diagnostic()
        .wrap_err("Failed to generate bindgen config")?;

    bindings
        .write_to_file(out_path.join(file))
        .into_diagnostic()
        .wrap_err("Failed to generate bindgen bindings")?;

    Ok(())
}

fn main() -> miette::Result<()> {
    do_bindgen("wrapper.h", "wrapper.rs")?;
    Ok(())
}
