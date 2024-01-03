use std::{env, path::PathBuf};

use bindgen::{
    callbacks::{DeriveInfo, TypeKind},
    CargoCallbacks,
};
use miette::{IntoDiagnostic, WrapErr};

#[derive(Debug)]
struct ConstDefaultCallbacks;

impl bindgen::callbacks::ParseCallbacks for ConstDefaultCallbacks {
    fn add_derives(&self, info: &DeriveInfo<'_>) -> Vec<String> {
        if info.kind == TypeKind::Struct {
            vec!["ConstDefault".to_string()]
        } else {
            vec![]
        }
    }
}

fn do_bindgen(metamod_variant: &str, header: &str, file: &str) -> miette::Result<()> {
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let metamod_include_paths: Vec<String> = [
        "",
        "hlsdk",
        "hlsdk/common",
        "hlsdk/dlls",
        "hlsdk/engine",
        "hlsdk/pm_shared",
        "metamod",
    ]
    .into_iter()
    .map(|path| format!("{metamod_variant}/{path}"))
    .flat_map(|path| vec!["-I".to_string(), path.to_owned()])
    .collect();

    let bindings = bindgen::Builder::default()
        .header(header)
        .clang_args(itertools::concat(vec![
            metamod_include_paths,
            vec!["-x".to_string(), "c++".to_string()],
        ]))
        .allowlist_file(format!(".*?{metamod_variant}.*"))
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
    do_bindgen(
        if cfg!(feature = "fallguys") {
            "metamod-fallguys"
        } else {
            "metamod-p"
        },
        "wrapper.h",
        "wrapper.rs",
    )?;
    Ok(())
}
