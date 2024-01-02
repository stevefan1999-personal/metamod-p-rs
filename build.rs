use bindgen::CargoCallbacks;
use miette::{IntoDiagnostic, WrapErr};
use std::{env, path::PathBuf};


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
            "-x", "c++"
        ])
        .blocklist_item("(_|P)?IMAGE_TLS_DIRECTORY(64)?")
        .vtable_generation(true)
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
