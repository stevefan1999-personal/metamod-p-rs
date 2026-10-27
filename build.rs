use std::{env, path::PathBuf, process::Command};

use miette::{IntoDiagnostic, WrapErr};

/// Header roots of one metamod flavour: the HLSDK copy it ships and the
/// directory that holds `meta_api.h`.
struct MetamodVariant {
    hlsdk:   &'static str,
    metamod: &'static str,
}

impl MetamodVariant {
    const FALLGUYS: Self = Self {
        hlsdk:   "metamod-fallguys/metamod/include/HLSDK",
        metamod: "metamod-fallguys/metamod/include",
    };
    const METAMOD_P: Self = Self {
        hlsdk:   "metamod-p/hlsdk",
        metamod: "metamod-p/metamod",
    };

    /// The HLSDK headers include each other without directory prefixes, so
    /// every SDK subdirectory has to be on the include path.
    fn include_args(&self) -> impl Iterator<Item = String> + '_ {
        ["", "/common", "/dlls", "/engine", "/pm_shared"]
            .into_iter()
            .map(|subdir| format!("{}{subdir}", self.hlsdk))
            .chain([self.metamod.to_string()])
            .map(|path| format!("-I{path}"))
    }

    /// A git dependency has the header trees as gitlinks, not as files. Fetch
    /// the include trees the same way CI does, and leave a developer checkout
    /// that already has them alone.
    fn ensure(&self) -> miette::Result<()> {
        let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").into_diagnostic()?);
        let marker = root.join(self.metamod).join("meta_api.h");
        if marker.is_file() {
            return Ok(());
        }
        let status = Command::new("bash")
            .arg(root.join("scripts/fetch-headers.sh"))
            .status()
            .into_diagnostic()
            .wrap_err("run scripts/fetch-headers.sh")?;
        if !status.success() {
            return Err(miette::miette!("header fetch failed"));
        }
        if !marker.is_file() {
            return Err(miette::miette!(
                "header fetch did not write {}",
                marker.display()
            ));
        }
        Ok(())
    }

    fn generate(&self, header: &str, file: &str) -> miette::Result<()> {
        let out_path = PathBuf::from(env::var("OUT_DIR").into_diagnostic()?);
        // Struct layouts depend on the pointer width, so clang must parse the
        // headers for the cargo target (i686 for Sven Co-op), not the host.
        let target = env::var("TARGET").into_diagnostic()?;

        let bindings = bindgen::Builder::default()
            .header(header)
            .clang_args(self.include_args())
            .clang_args([format!("--target={target}"), "-xc++".to_string()])
            .allowlist_file(format!(".*?({}|{}).*", self.hlsdk, self.metamod))
            .vtable_generation(true)
            .ctypes_prefix("::std::ffi")
            // The headers' doc comments are licence prose; rustdoc would run
            // their indented lines as doctests.
            .generate_comments(false)
            .derive_copy(true)
            .derive_default(true)
            .derive_hash(true)
            .generate_cstr(true)
            .array_pointers_in_arguments(true)
            .sort_semantically(true)
            .merge_extern_blocks(true)
            .layout_tests(true)
            .newtype_enum(".*")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .into_diagnostic()
            .wrap_err("Failed to generate bindgen config")?;

        bindings
            .write_to_file(out_path.join(file))
            .into_diagnostic()
            .wrap_err("Failed to generate bindgen bindings")
    }
}

fn main() -> miette::Result<()> {
    let variant = if cfg!(feature = "fallguys") {
        MetamodVariant::FALLGUYS
    } else {
        MetamodVariant::METAMOD_P
    };
    variant.ensure()?;
    variant.generate("wrapper.h", "wrapper.rs")
}
