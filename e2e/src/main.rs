//! Boots a Sven Co-op dedicated server with the example plugin, talks to its
//! console, and joins it with a fake client. The server install is only read
//! through symlinks.

mod client;

use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

const LINUX_METAMOD: Archive = Archive {
    name:   "metamod-fallguys-ubuntu-i386.zip",
    sha256: "552ae6d1faf184498f29d0c686515f56618eb55229970d7a687d8d5c74ed2d78",
    dll:    "metamod.so",
};
const WINDOWS_METAMOD: Archive = Archive {
    name:   "metamod-fallguys-windows-x86.zip",
    sha256: "cf677c95b2bdecd9eef5e6bd3d25cde7c919c1a33f32f715c70a5f8528a572f7",
    dll:    "metamod.dll",
};

struct Archive {
    name:   &'static str,
    sha256: &'static str,
    dll:    &'static str,
}

struct Options {
    server_dir: PathBuf,
    port:       u16,
    map:        String,
}

fn main() {
    if let Err(error) = run(options()) {
        eprintln!("e2e failed: {error}");
        std::process::exit(1);
    }
}

fn options() -> Options {
    let mut server_dir = env::var_os("SVEN_SERVER_DIR").map(PathBuf::from);
    let mut port = 27016_u16;
    let mut map = "hl_c01_a1".to_string();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server-dir" => server_dir = args.next().map(PathBuf::from),
            "--port" => {
                port = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .expect("--port needs a number")
            }
            "--map" => map = args.next().expect("--map needs a name"),
            other => panic!("unknown argument {other}"),
        }
    }
    Options {
        server_dir: server_dir.expect("set SVEN_SERVER_DIR or pass --server-dir"),
        port,
        map,
    }
}

fn run(options: Options) -> Result<(), String> {
    let archive = if cfg!(windows) {
        WINDOWS_METAMOD
    } else {
        LINUX_METAMOD
    };
    let binary = server_binary(&options.server_dir)?;
    let plugin = plugin_library()?;
    let root = env::temp_dir().join(format!("metamod-p-e2e-{}", options.port));
    if root.exists() {
        remove_farm(&root)?;
    }
    link_farm(&options.server_dir, &root)?;
    install_metamod(&root, &archive)?;
    install_plugin(&root, &plugin)?;

    let log_path = root.join("server.log");
    let log_file = fs::File::create(&log_path).map_err(|error| error.to_string())?;
    let mut command = Command::new(root.join(binary.file_name().unwrap()));
    command
        .current_dir(&root)
        .arg("-dll")
        .arg(format!("addons/metamod/dlls/{}", archive.dll))
        .arg("-console")
        .arg("-port")
        .arg(options.port.to_string())
        .arg("+sv_lan")
        .arg("1")
        .arg("+map")
        .arg(&options.map)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(
            log_file.try_clone().map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(log_file));
    if cfg!(unix) {
        let libraries = format!("{}:{}", root.display(), root.join("redist").display());
        command.env("LD_LIBRARY_PATH", libraries);
    }
    let mut server = command
        .spawn()
        .map_err(|error| format!("could not start the server: {error}"))?;
    let outcome = drive(&mut server, &root, &log_path, &options);
    let _ = writeln!(server.stdin.as_mut().unwrap(), "quit");
    let _ = server.kill();
    let _ = server.wait();
    let cleanup = remove_farm(&root);
    outcome.and(cleanup)
}

fn drive(
    server: &mut Child,
    root: &Path,
    log_path: &Path,
    options: &Options,
) -> Result<(), String> {
    wait_for_map(server, log_path)?;
    let stdin = server.stdin.as_mut().ok_or("the server has no console")?;
    writeln!(
        stdin,
        "sv_lan 1\nmeta list\nmeta cmds\nmeta cvars\nrs_hello from the e2e test"
    )
    .map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_secs(2));
    if let Err(error) = client::connect(format!("127.0.0.1:{}", options.port).parse().unwrap()) {
        let console = fs::read_to_string(log_path).unwrap_or_default();
        let tail = console
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("{error}\n--- server log ---\n{tail}"));
    }
    thread::sleep(Duration::from_secs(2));
    let library = if cfg!(windows) {
        "rust_hello.dll"
    } else {
        "rust_hello.so"
    };
    writeln!(
        stdin,
        "meta unload rust_hello\nmeta load addons/rust_hello/{library}\nrs_hello after reload"
    )
    .map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_secs(2));

    let console = fs::read_to_string(log_path).unwrap_or_default();
    let metamod = fs::read_to_string(root.join("metamod.log")).unwrap_or_default();
    let mut failed = false;
    check(
        &console,
        "the plugin attached",
        "[RSHELLO] attached at Startup",
        &mut failed,
    );
    check(
        &console,
        "meta list shows the plugin",
        "Rust Hello",
        &mut failed,
    );
    check(
        &console,
        "the map activated",
        &format!("[RSHELLO] map {} is active", options.map),
        &mut failed,
    );
    check(
        &metamod,
        "a precache post hook ran",
        "first precache:",
        &mut failed,
    );
    check(&console, "rs_hello ran", "from the e2e test", &mut failed);
    check(
        &console,
        "the fake client reached ClientConnect",
        &format!("\"{}\" connects from", client::NAME),
        &mut failed,
    );
    check(
        &console,
        "the plugin reloaded",
        "attached at AnyTime",
        &mut failed,
    );
    check(
        &console,
        "rs_hello ran after reload",
        "after reload",
        &mut failed,
    );
    if console.contains("panicked")
        || metamod.contains("panicked")
        || console.contains("Failed to attach")
    {
        eprintln!("FAILED: the plugin panicked or did not attach");
        failed = true;
    }
    if failed {
        eprintln!("--- server log ---\n{console}");
        eprintln!("--- metamod.log ---\n{metamod}");
        return Err("the server did not do what the example plugin promises".into());
    }
    println!("e2e ok: dedicated server, console commands, and the fake client");
    Ok(())
}

fn check(log: &str, label: &str, needle: &str, failed: &mut bool) {
    if log.contains(needle) {
        println!("ok: {label}");
    } else {
        eprintln!("FAILED: {label}");
        *failed = true;
    }
}

fn wait_for_map(server: &mut Child, log_path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if let Ok(log) = fs::read_to_string(log_path) {
            if log.contains("Started map") {
                return Ok(());
            }
        }
        match server.try_wait() {
            Ok(Some(status)) => {
                let log = fs::read_to_string(log_path).unwrap_or_default();
                return Err(format!(
                    "the server exited ({status}) before the map\n{log}"
                ));
            }
            Ok(None) => thread::sleep(Duration::from_secs(1)),
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("no map after 120s".into())
}

fn server_binary(server_dir: &Path) -> Result<PathBuf, String> {
    let candidates = if cfg!(windows) {
        ["svends.exe", "svends_i686.exe"]
    } else {
        ["svends_i686", "svends"]
    };
    candidates
        .into_iter()
        .map(|name| server_dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("no dedicated server binary in {}", server_dir.display()))
}

fn plugin_library() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
    let path = if cfg!(windows) {
        root.join("i686-pc-windows-msvc/release/example_plugin.dll")
    } else {
        root.join("i686-unknown-linux-gnu/release/libexample_plugin.so")
    };
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "build the example plugin first, missing {}",
            path.display()
        ))
    }
}

fn link_farm(server_dir: &Path, root: &Path) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(server_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        // svencoop itself must be a real directory. A symlink here would make
        // later writes (logs, addons) land in the shared install.
        if name == "svencoop" {
            let game = root.join("svencoop");
            fs::create_dir(&game).map_err(|error| error.to_string())?;
            for child in fs::read_dir(entry.path()).map_err(|error| error.to_string())? {
                let child = child.map_err(|error| error.to_string())?;
                symlink(&child.path(), &game.join(child.file_name()))?;
            }
        } else {
            symlink(&entry.path(), &root.join(name))?;
        }
    }
    for directory in ["svencoop_addon", "svencoop/logs", "svencoop/addons"] {
        replace_with_dir(&root.join(directory))?;
    }
    copy_over(server_dir, root, "steam_appid.txt")?;
    materialize_maps(server_dir, root)?;
    Ok(())
}

/// `maps` is written while the server runs, so it cannot be one symlink.
fn materialize_maps(server_dir: &Path, root: &Path) -> Result<(), String> {
    let source = server_dir.join("svencoop/maps");
    let destination = root.join("svencoop/maps");
    if !source.exists() {
        return Ok(());
    }
    unlink(&destination)?;
    fs::create_dir(&destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(&source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let to = destination.join(&name);
        if name == "graphs" || name == "soundcache" {
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                copy_dir(&entry.path(), &to)?;
            } else {
                fs::copy(entry.path(), &to).map_err(|error| error.to_string())?;
            }
        } else {
            symlink(&entry.path(), &to)?;
        }
    }
    Ok(())
}

fn install_metamod(root: &Path, archive: &Archive) -> Result<(), String> {
    let zip_path = root.join(archive.name);
    let bytes = download(&format!(
        "https://github.com/metamod-svencoop/metamod-svencoop/releases/download/v20261009a/{}",
        archive.name
    ))?;
    let digest = Sha256::digest(&bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if hex != archive.sha256 {
        return Err(format!("metamod archive hash is {hex}"));
    }
    fs::write(&zip_path, &bytes).map_err(|error| error.to_string())?;
    let file = fs::File::open(&zip_path).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().to_string();
        let Some(relative) = name.strip_prefix("build/addons/") else {
            continue;
        };
        if relative.is_empty() || relative.ends_with('/') {
            continue;
        }
        let destination = root.join("svencoop/addons").join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = fs::File::create(&destination).map_err(|error| error.to_string())?;
        io_copy(&mut entry, &mut output)?;
    }
    fs::write(
        root.join("svencoop/addons/metamod/plugins.ini"),
        "linux addons/rust_hello/rust_hello.so\nwin32 addons/rust_hello/rust_hello.dll\n",
    )
    .map_err(|error| error.to_string())
}

fn install_plugin(root: &Path, plugin: &Path) -> Result<(), String> {
    let name = if cfg!(windows) {
        "rust_hello.dll"
    } else {
        "rust_hello.so"
    };
    let destination = root.join("svencoop/addons/rust_hello").join(name);
    fs::create_dir_all(destination.parent().unwrap()).map_err(|error| error.to_string())?;
    fs::copy(plugin, destination).map_err(|error| error.to_string())?;
    Ok(())
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("curl")
        .args(["-fsSL", url])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("curl failed: {error}"))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(bytes)
    } else {
        Err(format!("curl exited {status}"))
    }
}

fn copy_over(server_dir: &Path, root: &Path, relative: &str) -> Result<(), String> {
    let source = server_dir.join(relative);
    if !source.exists() {
        return Ok(());
    }
    let destination = root.join(relative);
    unlink(&destination)?;
    if source.is_dir() {
        copy_dir(&source, &destination)
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::copy(source, destination)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let to = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn replace_with_dir(path: &Path) -> Result<(), String> {
    unlink(path)?;
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn unlink(path: &Path) -> Result<(), String> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path).map_err(|error| error.to_string())
    } else {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    }
}

fn remove_farm(root: &Path) -> Result<(), String> {
    // Walk with symlink_metadata so a directory symlink is unlinked, not
    // followed. remove_dir_all on Windows can delete the symlink target.
    remove_tree(root)
}

fn remove_tree(path: &Path) -> Result<(), String> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return fs::remove_file(path).map_err(|error| error.to_string());
    }
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        remove_tree(&entry.map_err(|error| error.to_string())?.path())?;
    }
    fs::remove_dir(path).map_err(|error| error.to_string())
}

fn symlink(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination).map_err(|error| error.to_string())
    }
    #[cfg(windows)]
    {
        if source.is_dir() {
            std::os::windows::fs::symlink_dir(source, destination)
        } else {
            std::os::windows::fs::symlink_file(source, destination)
        }
        .map_err(|error| error.to_string())
    }
}

fn io_copy(reader: &mut impl Read, writer: &mut impl Write) -> Result<(), String> {
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if size == 0 {
            return Ok(());
        }
        writer
            .write_all(&buffer[..size])
            .map_err(|error| error.to_string())?;
    }
}
