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
    /// A plugin other than the example. Its checks are `--expect` substrings.
    plugin:     Option<PathBuf>,
    addon:      Option<String>,
    expects:    Vec<String>,
}

struct PluginSpec {
    source:    PathBuf,
    addon:     String,
    file_name: String,
    example:   bool,
    expects:   Vec<String>,
}

impl PluginSpec {
    fn stem(&self) -> &str {
        self.file_name.split('.').next().unwrap_or(&self.file_name)
    }

    fn relative(&self) -> String {
        format!("addons/{}/{}", self.addon, self.file_name)
    }

    fn ini(&self) -> String {
        let stem = self.stem();
        format!(
            "linux addons/{addon}/{stem}.so\nwin32 addons/{addon}/{stem}.dll\n",
            addon = self.addon
        )
    }
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
    let mut plugin = None;
    let mut addon = None;
    let mut expects = Vec::new();
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
            "--plugin" => plugin = args.next().map(PathBuf::from),
            "--addon" => addon = args.next(),
            "--expect" => expects.push(args.next().expect("--expect needs a substring")),
            other => panic!("unknown argument {other}"),
        }
    }
    Options {
        server_dir: server_dir.expect("set SVEN_SERVER_DIR or pass --server-dir"),
        port,
        map,
        plugin,
        addon,
        expects,
    }
}

fn plugin_spec(options: &Options) -> Result<PluginSpec, String> {
    let Some(source) = options.plugin.clone() else {
        return Ok(PluginSpec {
            source:    plugin_library()?,
            addon:     "rust_hello".to_string(),
            file_name: if cfg!(windows) {
                "rust_hello.dll"
            } else {
                "rust_hello.so"
            }
            .to_string(),
            example:   true,
            expects:   Vec::new(),
        });
    };
    let mut file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("plugin.so")
        .to_string();
    if let Some(stripped) = file_name.strip_prefix("lib") {
        file_name = stripped.to_string();
    }
    if cfg!(windows) {
        let stem = file_name.split('.').next().unwrap_or("plugin");
        file_name = format!("{stem}.dll");
    }
    let stem = file_name.split('.').next().unwrap_or("plugin").to_string();
    Ok(PluginSpec {
        source,
        addon: options.addon.clone().unwrap_or(stem),
        file_name,
        example: false,
        expects: options.expects.clone(),
    })
}

fn run(options: Options) -> Result<(), String> {
    let archive = if cfg!(windows) {
        WINDOWS_METAMOD
    } else {
        LINUX_METAMOD
    };
    let binary = server_binary(&options.server_dir)?;
    let plugin = plugin_spec(&options)?;
    let root = farm_dir(&options.server_dir, options.port);
    if root.exists() {
        remove_farm(&root)?;
    }
    link_farm(&options.server_dir, &root)?;
    install_metamod(&root, &archive, &plugin)?;
    install_plugin(&root, &plugin)?;
    // svends.exe does not read a stdin pipe. server.cfg runs at map start,
    // and its output is the console log the checks read.
    #[cfg(windows)]
    install_console_cfg(&root, &console_script(&plugin))?;

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
    let program = root.join(binary.file_name().unwrap());
    let mut server = command
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", program.display()))?;
    let outcome = drive(&mut server, &root, &log_path, &options, &plugin);
    let _ = writeln!(server.stdin.as_mut().unwrap(), "quit");
    let _ = server.kill();
    let _ = server.wait();
    let cleanup = remove_farm(&root);
    outcome.and(cleanup)
}

fn console_script(plugin: &PluginSpec) -> String {
    let mut lines = vec![
        "sv_lan 1".to_string(),
        "meta list".to_string(),
        "meta cmds".to_string(),
        "meta cvars".to_string(),
    ];
    if plugin.example {
        lines.push("rs_hello from the e2e test".to_string());
    }
    lines.push(format!("meta unload {}", plugin.stem()));
    lines.push(format!("meta load {}", plugin.relative()));
    if plugin.example {
        lines.push("rs_hello after reload".to_string());
    }
    lines.join("\n")
}

fn drive(
    server: &mut Child,
    root: &Path,
    log_path: &Path,
    options: &Options,
    plugin: &PluginSpec,
) -> Result<(), String> {
    wait_for_map(server, log_path)?;
    let stdin = server.stdin.as_mut().ok_or("the server has no console")?;
    let script = console_script(plugin);
    let (before, after) = script
        .split_once("meta unload")
        .map(|(before, after)| (before.to_string(), format!("meta unload{after}")))
        .unwrap_or((script.clone(), String::new()));
    writeln!(stdin, "{before}").map_err(|error| error.to_string())?;
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
    writeln!(stdin, "{after}").map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_secs(2));

    let console = fs::read_to_string(log_path).unwrap_or_default();
    let metamod = fs::read_to_string(root.join("metamod.log")).unwrap_or_default();
    let mut failed = false;
    if plugin.example {
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
    } else {
        let combined = format!("{console}\n{metamod}");
        for needle in &plugin.expects {
            check(&combined, needle, needle, &mut failed);
        }
        check(
            &console,
            "the fake client reached the server",
            client::NAME,
            &mut failed,
        );
    }
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

/// Hard links cannot cross volumes, so the Windows farm sits beside the
/// install. Symlinks have no such limit and stay in the temp directory.
#[cfg(windows)]
fn farm_dir(server_dir: &Path, port: u16) -> PathBuf {
    server_dir
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("metamod-p-e2e-{port}"))
}

#[cfg(not(windows))]
fn farm_dir(_server_dir: &Path, port: u16) -> PathBuf {
    env::temp_dir().join(format!("metamod-p-e2e-{port}"))
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

fn install_metamod(root: &Path, archive: &Archive, plugin: &PluginSpec) -> Result<(), String> {
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
        plugin.ini(),
    )
    .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn install_console_cfg(root: &Path, commands: &str) -> Result<(), String> {
    let path = root.join("svencoop/server.cfg");
    let original = fs::read_to_string(&path).unwrap_or_default();
    if path.exists() {
        // A hard link shares the install's file. Drop the link, then write a
        // new file so the commands do not land in the shared server.cfg.
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        let mut permissions = metadata.permissions();
        // This file is our copy. On Windows the call clears the readonly
        // attribute. The Unix meaning (world writable) does not apply here.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
        unlink(&path)?;
    }
    let script = format!("{original}\n{commands}\n");
    fs::write(&path, script).map_err(|error| format!("write {}: {error}", path.display()))
}

fn install_plugin(root: &Path, plugin: &PluginSpec) -> Result<(), String> {
    let destination = root
        .join("svencoop/addons")
        .join(&plugin.addon)
        .join(&plugin.file_name);
    fs::create_dir_all(destination.parent().unwrap()).map_err(|error| error.to_string())?;
    fs::copy(&plugin.source, destination).map_err(|error| error.to_string())?;
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
    if is_link(&metadata) || metadata.is_file() {
        // A junction is a directory reparse point. remove_file does not
        // remove it, and remove_dir removes the junction without its target.
        fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(|error| format!("unlink {}: {error}", path.display()))
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
    if is_link(&metadata) || metadata.is_file() {
        return fs::remove_file(path)
            .or_else(|_| fs::remove_dir(path))
            .map_err(|error| format!("unlink {}: {error}", path.display()));
    }
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        remove_tree(&entry.map_err(|error| error.to_string())?.path())?;
    }
    fs::remove_dir(path).map_err(|error| error.to_string())
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    // Directory junctions are reparse points, and Rust does not report them
    // as symlinks. Following one would delete the dedicated-server install.
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn symlink(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination).map_err(|error| {
            format!(
                "symlink {} -> {}: {error}",
                destination.display(),
                source.display()
            )
        })
    }
    // CreateSymbolicLinkW is denied on the GitHub Windows runner. A junction
    // or a hard link does not need that privilege.
    #[cfg(windows)]
    {
        let metadata = fs::symlink_metadata(source)
            .map_err(|error| format!("stat {}: {error}", source.display()))?;
        if metadata.is_dir() {
            junction(source, destination)
        } else if destination
            .file_name()
            .is_some_and(|name| name == "server.cfg")
        {
            // A hard link would make the console commands overwrite the
            // install.
            fs::copy(source, destination).map(|_| ()).map_err(|error| {
                format!(
                    "copy {} -> {}: {error}",
                    destination.display(),
                    source.display()
                )
            })
        } else {
            fs::hard_link(source, destination)
                .or_else(|_| fs::copy(source, destination).map(|_| ()))
                .map_err(|error| {
                    format!(
                        "link {} -> {}: {error}",
                        destination.display(),
                        source.display()
                    )
                })
        }
    }
}

#[cfg(windows)]
fn junction(target: &Path, link: &Path) -> Result<(), String> {
    // cmd treats a `/` inside an unquoted path as a switch.
    let link = link.as_os_str().to_string_lossy().replace('/', "\\");
    let target = target.as_os_str().to_string_lossy().replace('/', "\\");
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J", &link, &target])
        .output()
        .map_err(|error| format!("mklink: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr);
        Err(format!("junction {link} -> {target}: {}", message.trim()))
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
