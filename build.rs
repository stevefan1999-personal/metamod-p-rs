use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

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
    /// the include tree with `git` itself. `bash` on a Windows runner is the
    /// WSL stub, which is not installed there.
    fn ensure(&self) -> miette::Result<()> {
        let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").into_diagnostic()?);
        let marker = root.join(self.metamod).join("meta_api.h");
        if marker.is_file() {
            return Ok(());
        }
        if self.hlsdk.contains("fallguys") {
            fetch_fallguys(&root)?;
        } else {
            fetch_metamod_p(&root)?;
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

fn git(cwd: &Path, args: &[String]) -> miette::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .into_diagnostic()
        .wrap_err("git")?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(miette::miette!("git {args:?} failed: {detail}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn fetch_metamod_p(root: &Path) -> miette::Result<()> {
    let dest = root.join("metamod-p");
    if dest.exists() {
        fs::remove_dir_all(&dest).into_diagnostic()?;
    }
    git(
        root,
        &[
            "clone".into(),
            "--depth".into(),
            "1".into(),
            "https://github.com/Bots-United/metamod-p.git".into(),
            dest.to_string_lossy().into_owned(),
        ],
    )?;
    Ok(())
}

fn fetch_fallguys(root: &Path) -> miette::Result<()> {
    let rev = git(root, &["rev-parse".into(), "HEAD:metamod-fallguys".into()])?;
    let probe = env::temp_dir().join(format!("metamod-headers-{}", std::process::id()));
    let _ = fs::remove_dir_all(&probe);
    fs::create_dir_all(&probe).into_diagnostic()?;
    git(&probe, &["init".into(), "-q".into()])?;
    git(
        &probe,
        &[
            "fetch".into(),
            "--depth".into(),
            "1".into(),
            "https://github.com/hzqst/metamod-fallguys.git".into(),
            rev,
        ],
    )?;
    let headers = git(&probe, &["rev-parse".into(), "FETCH_HEAD:metamod".into()])?;
    let _ = fs::remove_dir_all(&probe);
    let dest = root.join("metamod-fallguys/metamod");
    fs::create_dir_all(root.join("metamod-fallguys")).into_diagnostic()?;
    if dest.exists() {
        fs::remove_dir_all(&dest).into_diagnostic()?;
    }
    git(
        root,
        &[
            "clone".into(),
            "--filter=blob:none".into(),
            "--sparse".into(),
            "https://github.com/metamod-fallguys/metamod.git".into(),
            dest.to_string_lossy().into_owned(),
        ],
    )?;
    git(
        &dest,
        &["sparse-checkout".into(), "set".into(), "include".into()],
    )?;
    git(
        &dest,
        &[
            "fetch".into(),
            "--depth".into(),
            "1".into(),
            "origin".into(),
            headers,
        ],
    )?;
    git(
        &dest,
        &["checkout".into(), "--detach".into(), "FETCH_HEAD".into()],
    )?;
    Ok(())
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
