//! Horizon VR installer: a small window like Vivecraft's. Copies the prebuilt game beside this
//! program to the chosen folder, prepares the game files from your own Minecraft 26.3, and adds a
//! desktop shortcut. It finds the Horizon launcher's (or the Minecraft Launcher's) Minecraft 26.3
//! and server jar by itself. `-Silent` installs with what it found, without the window. Install
//! also copies this installer into the install folder (a Start menu shortcut opens it), where its
//! Update button fetches the latest release from GitHub and updates the game and itself.
#![windows_subsystem = "windows"]

mod gui;

use serde_json::Value;
use sha1::Digest;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use std::{env, fs};

pub type Say<'a> = &'a dyn Fn(&str);
pub type R<T> = Result<T, String>;

const EXE: &str = "HorizonVR-Installer.exe";
const NO_WINDOW: u32 = 0x0800_0000;

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(env::var_os(name).unwrap_or_default())
}

/// The folder this program is in.
pub fn here() -> PathBuf {
    env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
}

/// The Horizon launcher's data folder. HORIZONVR_LAUNCHER_DATA points the tests elsewhere.
fn launcher() -> PathBuf {
    env::var_os("HORIZONVR_LAUNCHER_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env_dir("APPDATA").join("horizon-launcher"))
}

/// HORIZONVR_LAUNCHER points the tests away from the player's launcher.
fn launcher_exe() -> PathBuf {
    env::var_os("HORIZONVR_LAUNCHER")
        .map(PathBuf::from)
        .unwrap_or_else(|| env_dir("LOCALAPPDATA").join(r"Programs\Horizon Launcher\horizon-launcher.exe"))
}

/// Where releases come from. HORIZONVR_RELEASES points the tests at a local copy.
fn releases() -> String {
    env::var("HORIZONVR_RELEASES")
        .unwrap_or_else(|_| "https://api.github.com/repos/v1nx0/horizon-vivecraft-vr/releases/latest".into())
}

fn start_menu_link() -> PathBuf {
    env_dir("APPDATA").join(r"Microsoft\Windows\Start Menu\Programs\Horizon VR Installer.lnk")
}

fn desktop_link() -> PathBuf {
    dirs::desktop_dir().unwrap_or_else(|| env_dir("USERPROFILE").join("Desktop")).join("Horizon VR.lnk")
}

/// A Minecraft folder with 26.3 downloaded: the Horizon launcher's first, then the official one.
pub fn find_minecraft() -> PathBuf {
    let official = env_dir("APPDATA").join(".minecraft");
    [launcher().join("minecraft"), official.clone()]
        .into_iter()
        .find(|d| d.join(r"versions\26.3\26.3.jar").exists())
        .unwrap_or(official)
}

fn json_file(path: &Path) -> Option<Value> {
    serde_json::from_str(fs::read_to_string(path).ok()?.trim_start_matches('\u{feff}')).ok()
}

/// The pinned 26.3 server jar's SHA-1: from the lock beside this program (in files\ in the
/// unzipped release, at the top in an install folder), or the release an install uses.
static SERVER_SHA: Mutex<Option<String>> = Mutex::new(None);

fn server_sha() -> Option<String> {
    let mut s = SERVER_SHA.lock().unwrap_or_else(|e| e.into_inner());
    if s.is_none() {
        let h = here();
        *s = [h.join(r"files\research-lock.json"), h.join("research-lock.json")]
            .iter()
            .find_map(|p| json_file(p)?["minecraft"]["server_sha1"].as_str().map(str::to_owned));
    }
    s.clone()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_hash<D: Digest>(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut d = D::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            return Some(hex(&d.finalize()));
        }
        d.update(&buf[..n]);
    }
}

pub fn is_server_jar(path: &Path) -> bool {
    let Some(sha) = server_sha() else { return false };
    path.is_file() && file_hash::<sha1::Sha1>(path).is_some_and(|h| h.eq_ignore_ascii_case(&sha))
}

/// The 26.3 server jar: the one the Horizon launcher downloaded, else a matching one in Downloads.
pub fn find_server_jar() -> String {
    let l = launcher();
    let mut candidates = vec![l.join(r"server-jars\26.3.jar")];
    if let Ok(vs) = fs::read_dir(l.join("versions")) {
        candidates.extend(vs.flatten().map(|v| v.path().join(r".local\minecraft\26.3\jars\server.jar")));
    }
    if let Ok(ds) = fs::read_dir(env_dir("USERPROFILE").join("Downloads")) {
        candidates.extend(ds.flatten().map(|e| e.path()).filter(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar"))
                && fs::metadata(p).is_ok_and(|m| m.len() > 30 << 20)
        }));
    }
    candidates
        .into_iter()
        .find(|j| is_server_jar(j))
        .map(|j| j.display().to_string())
        .unwrap_or_default()
}

/// Run from an install folder (its copy of the installer, its version.txt): that folder.
pub fn is_install_folder(dir: &Path) -> bool {
    dir.join("horizon-client.exe").exists() || dir.join("version.txt").exists()
}

pub fn default_install_dir() -> PathBuf {
    let h = here();
    if is_install_folder(&h) { h } else { env_dir("LOCALAPPDATA").join("HorizonVR") }
}

/// A version like 0.3.2, compared part by part.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(Vec<u32>);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Option<Vec<u32>> = s.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect();
        parts.filter(|p| (2..=4).contains(&p.len())).map(Version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s: Vec<String> = self.0.iter().map(u32::to_string).collect();
        f.write_str(&s.join("."))
    }
}

/// A version file's version (version.txt), or none.
pub fn read_version(path: &Path) -> Option<Version> {
    Version::parse(fs::read_to_string(path).ok()?.trim_start_matches('\u{feff}'))
}

pub struct Latest {
    pub version: Version,
    url: String,
    sha256: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).build()
}

/// The latest release: its version and the installer zip's address and SHA-256.
pub fn get_latest() -> R<Latest> {
    let r: Value = agent()
        .get(&releases())
        .set("User-Agent", "HorizonVR-Installer")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    let tag = r["tag_name"].as_str().unwrap_or_default();
    let asset = r["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["name"] == "HorizonVR-Installer.zip"))
        .ok_or_else(|| format!("The latest release ({tag}) has no HorizonVR-Installer.zip."))?;
    Ok(Latest {
        version: Version::parse(tag).ok_or_else(|| format!("The latest release's tag ({tag}) is not a version."))?,
        url: asset["browser_download_url"].as_str().unwrap_or_default().to_owned(),
        sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_owned),
    })
}

fn download(url: &str, to: &Path) -> R<()> {
    let resp = agent().get(url).set("User-Agent", "HorizonVR-Installer").call().map_err(|e| e.to_string())?;
    let mut f = fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    std::io::copy(&mut resp.into_reader(), &mut f).map_err(|e| e.to_string())?;
    Ok(())
}

/// The folder in `dir` (or below) that holds a release's files\ with the game in it.
fn release_root(dir: &Path) -> Option<PathBuf> {
    if dir.join(r"files\horizon-client.exe").exists() {
        return Some(dir.to_path_buf());
    }
    fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| release_root(&e.path()))
}

/// Downloads and unzips a release; returns the folder holding its installer and files\.
pub fn get_release(latest: &Latest, say: Say) -> R<PathBuf> {
    let work = env::temp_dir().join(format!("horizonvr-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let zip = work.join("HorizonVR-Installer.zip");
    say(&format!("Downloading Horizon VR {}...", latest.version));
    download(&latest.url, &zip)?;
    if let Some(sha) = &latest.sha256
        && !file_hash::<sha2::Sha256>(&zip).is_some_and(|h| h.eq_ignore_ascii_case(sha))
    {
        return Err("The downloaded release did not match its checksum. Try again.".into());
    }
    let f = fs::File::open(&zip).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(f).and_then(|mut z| z.extract(&work)).map_err(|e| format!("Unzipping the release failed: {e}"))?;
    release_root(&work).ok_or_else(|| "The downloaded release is missing its game files.".into())
}

/// Runs a program, passing each output line to `say`, and returns its exit code.
pub fn run(exe: &Path, args: &[&str], dir: &Path, say: Say) -> i32 {
    let child = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            say(&format!("{}: {e}", exe.display()));
            return -1;
        }
    };
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let readers: Vec<_> = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|s| {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(s).split(b'\n').map_while(Result::ok) {
                let _ = tx.send(String::from_utf8_lossy(&line).trim_end_matches('\r').to_owned());
            }
        })
    })
    .collect();
    drop(tx);
    for line in rx {
        say(&line);
    }
    readers.into_iter().for_each(|r| drop(r.join()));
    child.wait().ok().and_then(|s| s.code()).unwrap_or(-1)
}

/// A launcher command's `--json` output.
fn launcher_json(args: &[&str]) -> Option<Value> {
    let out = Command::new(launcher_exe()).args(args).stdin(Stdio::null()).creation_flags(NO_WINDOW).output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Full paths without quotes (a command line can leave one after a trailing backslash) or a
/// trailing backslash.
pub fn full_path(p: &str) -> PathBuf {
    let p = p.trim().trim_matches('"').trim();
    let abs = std::path::absolute(p).unwrap_or_else(|_| PathBuf::from(p));
    let s = abs.display().to_string();
    let t = s.trim_end_matches('\\');
    // A drive root keeps its backslash.
    PathBuf::from(if t.ends_with(':') { s.as_str() } else { t })
}

/// A running game locks its exe; asking the file works for every spelling of the path.
fn assert_closed(dest: &Path) -> R<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let exe = dest.join("horizon-client.exe");
    if !exe.exists() {
        return Ok(());
    }
    // Windows keeps the lock a moment after the game exits: give it 5 seconds.
    for _ in 0..10 {
        if fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&exe).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("Horizon VR is running from that folder. Close it first.".into())
}

/// What the installer puts in the install folder. Everything else there (saves, options.txt,
/// screenshots, resource packs, mods) is the player's and stays. The last two are older installs'.
const INSTALLED: &[&str] = &[
    "horizon-client.exe", "horizon-prepare.exe", "openxr_loader.dll", "research-lock.json", "LICENSE.txt",
    "Play Horizon VR.bat", "exporters", r"mods\horizon", r"crates\horizon-prepare", ".local", ".horizon-cache",
    "version.txt", "horizon-base.txt", EXE, "HorizonVR-Installer.ps1", "Horizon VR Installer.bat",
];

fn remove(path: &Path) -> R<()> {
    let r = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    r.map_err(|e| format!("{}: {e}", path.display()))
}

fn is_empty_dir(p: &Path) -> bool {
    fs::read_dir(p).is_ok_and(|mut d| d.next().is_none())
}

/// Removes Horizon VR: its files and the desktop shortcut, keeping the player's worlds and settings.
pub fn uninstall(dest: &str, say: Say) -> R<()> {
    let dest = full_path(dest);
    if !dest.join("horizon-client.exe").exists() || !dest.join("research-lock.json").exists() {
        // Files gone already (an older uninstall kept the launcher instance): remove just that.
        if remove_launcher_instance(&dest, say)? {
            say("Horizon VR is uninstalled.");
            return Ok(());
        }
        return Err(format!("Horizon VR is not installed in {}.", dest.display()));
    }
    assert_closed(&dest)?;
    say(&format!("Removing Horizon VR from {}...", dest.display()));
    // The running installer can't delete its own exe: Windows lets it be renamed, then it goes later.
    let me = env::current_exe().ok().and_then(|p| fs::canonicalize(p).ok());
    for name in INSTALLED {
        let path = dest.join(name);
        if !path.exists() {
            continue;
        }
        if me.is_some() && fs::canonicalize(&path).ok() == me {
            let gone = env::temp_dir().join(format!("horizonvr-old-{}.exe", std::process::id()));
            fs::rename(&path, gone).map_err(|e| format!("{}: {e}", path.display()))?;
            continue;
        }
        remove(&path)?;
    }
    // The parents of the nested ones, when nothing else is in them.
    for name in ["mods", "crates"] {
        let path = dest.join(name);
        if is_empty_dir(&path) {
            let _ = fs::remove_dir(&path);
        }
    }
    if let Err(e) = remove_launcher_instance(&dest, say) {
        say(&format!("The Horizon launcher instance was not removed: {e}"));
    }
    for link in [desktop_link(), start_menu_link()] {
        let _ = fs::remove_file(link);
    }
    if is_empty_dir(&dest) {
        let _ = fs::remove_dir(&dest);
        say("Horizon VR is uninstalled.");
    } else {
        say(&format!("Horizon VR is uninstalled. Your worlds and settings are still in {}.", dest.display()));
    }
    Ok(())
}

/// Reinstalls over a broken install: fresh game files and freshly prepared Minecraft files.
pub fn repair(dest: &str, mc: &str, server: &str, say: Say) -> R<()> {
    let d = full_path(dest);
    assert_closed(&d)?;
    let local = d.join(".local");
    if local.exists() {
        say("Removing the prepared game files...");
        remove(&local)?;
    }
    install(dest, mc, server, say, None)
}

/// Updates an install to the latest release, the installer in it too. False when already latest.
pub fn update(dest: &str, mc: &str, server: &str, say: Say) -> R<bool> {
    let d = full_path(dest);
    let have = read_version(&d.join("version.txt"));
    let latest = get_latest()?;
    if let Some(have) = have.filter(|h| *h >= latest.version) {
        say(&format!("Horizon VR {have} is up to date."));
        return Ok(false);
    }
    assert_closed(&d)?;
    let src = get_release(&latest, say)?;
    install(dest, mc, server, say, Some(src))?;
    Ok(true)
}

fn copy_all(from: &Path, to: &Path) -> R<()> {
    if from.is_dir() {
        fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for e in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
            copy_all(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(drop).map_err(|e| format!("{}: {e}", to.display()))
    }
}

fn shortcut(link: &Path, target: &Path, dir: &Path, icon: &Path) -> R<()> {
    let mut l = mslnk::ShellLink::new(target).map_err(|e| e.to_string())?;
    l.set_working_dir(Some(dir.display().to_string()));
    l.set_icon_location(Some(icon.display().to_string()));
    if let Some(p) = link.parent() {
        let _ = fs::create_dir_all(p);
    }
    l.create_lnk(link).map_err(|e| format!("{}: {e}", link.display()))
}

/// The whole install. `src`: the folder with the release's files\ (default: beside this program;
/// the installer copy in an install folder has none, so it fetches the latest release).
pub fn install(dest: &str, mc: &str, server: &str, say: Say, src: Option<PathBuf>) -> R<()> {
    let dest = full_path(dest);
    let mc = full_path(mc);
    let mut server = server.trim().trim_matches('"').to_owned();
    let h = here();
    let mut src = src.unwrap_or_else(|| h.clone());
    // Already installed (this version, game files prepared): nothing to copy or prepare again.
    // Only a missing launcher instance is added. Repair removes .local first, so it still runs.
    // The copy in an install folder has no files\ beside it: its own folder's version.
    let want = [src.join(r"files\version.txt"), src.join("version.txt")].iter().find_map(|p| read_version(p));
    if let Some(want) = want.filter(|w| {
        Some(w) == read_version(&dest.join("version.txt")).as_ref()
            && dest.join("horizon-client.exe").exists()
            && dest.join(r".local\horizon-client\26.3").exists()
    }) {
        let has_instance = !launcher_exe().exists() || has_launcher_instance();
        if !has_instance && let Err(e) = add_launcher_instance(&dest, say) {
            say(&format!("The Horizon launcher instance was not set up: {e}"));
        }
        say(&format!(
            "Horizon VR {want} is already installed in {}. (Use Repair to install it again.)",
            dest.display()
        ));
        return Ok(());
    }
    if !src.join(r"files\horizon-client.exe").exists() {
        // An install folder fetches the release, even with its game deleted.
        if !is_install_folder(&h) {
            return Err("The game files are missing. Unzip the whole HorizonVR-Installer.zip and run the installer from the unzipped folder.".into());
        }
        src = get_release(&get_latest()?, say)?;
    }
    if !mc.join(r"versions\26.3\26.3.jar").exists() {
        return Err("Minecraft 26.3 is not downloaded in that Minecraft folder. Play 26.3 once in the Horizon launcher or the Minecraft Launcher first.".into());
    }
    // The pinned hash from the release itself, when this program had no lock beside it.
    {
        let mut s = SERVER_SHA.lock().unwrap_or_else(|e| e.into_inner());
        if s.is_none() {
            *s = json_file(&src.join(r"files\research-lock.json"))
                .and_then(|j| j["minecraft"]["server_sha1"].as_str().map(str::to_owned));
        }
    }
    if server.is_empty() {
        server = find_server_jar();
    }
    let base_file = [src.join(r"files\horizon-base.txt"), src.join("horizon-base.txt")].into_iter().find(|p| p.exists());
    if (server.is_empty() || !is_server_jar(Path::new(&server)))
        && let Some(bf) = &base_file
        && launcher_exe().exists()
    {
        // No jar yet: the launcher installs and prepares the nightly this build is on, which
        // downloads it (and the version then plays in the launcher too).
        let want = fs::read_to_string(bf).unwrap_or_default();
        let base = resolve_base(want.trim(), &dest, say)?;
        say(&format!("Preparing Horizon {base} in the Horizon launcher (a few minutes)..."));
        run(&launcher_exe(), &["prepare", "--version", &base], &existing(&dest), say);
        server = find_server_jar();
    }
    if server.is_empty() || !is_server_jar(Path::new(&server)) {
        return Err("Choose the Minecraft 26.3 server jar (download it from minecraft.net). Another version does not work.".into());
    }
    assert_closed(&dest)?;

    say(&format!("Copying Horizon VR to {}...", dest.display()));
    fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    // Replace what an earlier install shipped, so no file a newer version dropped stays behind;
    // .local (the prepared game files) is kept. Older installs' PowerShell installer goes.
    for old in ["HorizonVR-Installer.ps1", "Horizon VR Installer.bat"] {
        let _ = fs::remove_file(dest.join(old));
    }
    let me = env::current_exe().ok().and_then(|p| fs::canonicalize(p).ok());
    for e in fs::read_dir(src.join("files")).map_err(|e| e.to_string())?.flatten() {
        let old = dest.join(e.file_name());
        if old.exists() {
            // The installer running from this folder: moved aside (Windows allows that), then replaced.
            if me.is_some() && fs::canonicalize(&old).ok() == me {
                let aside = env::temp_dir().join(format!("horizonvr-old-{}.exe", std::process::id()));
                let _ = fs::remove_file(&aside);
                fs::rename(&old, aside).map_err(|e| format!("{}: {e}", old.display()))?;
            } else {
                remove(&old)?;
            }
        }
        copy_all(&e.path(), &old)?;
    }
    // A release made before the installer shipped in files\: its exe beside files\.
    if !dest.join(EXE).exists() && src.join(EXE).exists() {
        copy_all(&src.join(EXE), &dest.join(EXE))?;
    }

    // The Minecraft Launcher may rewrite its 26.3 version file; fetch Mojang's own, as the
    // preparer asks, and check it against the pinned SHA-1.
    let sha = json_file(&dest.join("research-lock.json"))
        .and_then(|j| j["minecraft"]["manifest_sha1"].as_str().map(str::to_owned))
        .ok_or("The release's research-lock.json has no manifest_sha1.")?;
    let manifest = dest.join(r".local\minecraft\26.3\manifest.json");
    if !file_hash::<sha1::Sha1>(&manifest).is_some_and(|h| h.eq_ignore_ascii_case(&sha)) {
        say("Downloading the Minecraft 26.3 version file from Mojang...");
        fs::create_dir_all(manifest.parent().expect("has a parent")).map_err(|e| e.to_string())?;
        download(&format!("https://piston-meta.mojang.com/v1/packages/{sha}/26.3.json"), &manifest)?;
        if !file_hash::<sha1::Sha1>(&manifest).is_some_and(|h| h.eq_ignore_ascii_case(&sha)) {
            let _ = fs::remove_file(&manifest);
            return Err("The downloaded version file did not match.".into());
        }
    }

    say("Preparing the game files from your Minecraft (about a minute)...");
    let prepare = dest.join("horizon-prepare.exe");
    let mcs = mc.display().to_string();
    if run(&prepare, &["--prepare", "--minecraft-dir", &mcs, "--server-jar", &server], &dest, say) != 0 {
        return Err("Preparing the game files failed (see the log).".into());
    }
    if run(&prepare, &["sounds", "--minecraft-dir", &mcs], &dest, say) != 0 {
        say("Sounds were not copied; the game runs without them.");
    }

    let icon = dest.join("horizon-client.exe");
    shortcut(&desktop_link(), &dest.join("Play Horizon VR.bat"), &dest, &icon)?;
    shortcut(&start_menu_link(), &dest.join(EXE), &dest, &dest.join(EXE))?;
    if let Err(e) = add_launcher_instance(&dest, say) {
        say(&format!("The Horizon launcher instance was not set up: {e}"));
    }
    say("Done. Start SteamVR, then open \"Horizon VR\" on your desktop (or the Horizon VR instance in the Horizon launcher).");
    Ok(())
}

/// A folder to run in: `dest` once it exists, else its nearest existing parent.
fn existing(dest: &Path) -> PathBuf {
    dest.ancestors().find(|p| p.is_dir()).unwrap_or(Path::new(".")).to_path_buf()
}

/// The launcher's installed Horizon versions (not imported or native builds), newest first.
fn installed_bases() -> Vec<String> {
    let v = launcher_json(&["versions", "--installed", "--json"]).unwrap_or_default();
    v["installed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|n| !n.starts_with("imported-") && !n.starts_with("native-"))
        .map(str::to_owned)
        .collect()
}

/// The launcher version to import the game on: the one it was built on (horizon-base.txt) when
/// it is installed or still downloadable. Nightlies leave GitHub after a while, so then any
/// installed Horizon version, else the latest release: the base only lends its game files.
pub fn resolve_base(want: &str, dest: &Path, say: Say) -> R<String> {
    let versions = launcher().join("versions");
    if versions.join(want).exists() {
        return Ok(want.to_owned());
    }
    let dir = existing(dest);
    say(&format!("Installing Horizon {want} in the Horizon launcher..."));
    if run(&launcher_exe(), &["install", want], &dir, say) == 0 && versions.join(want).exists() {
        return Ok(want.to_owned());
    }
    if let Some(have) = installed_bases().into_iter().find(|v| versions.join(v).exists()) {
        say(&format!("{want} is gone from GitHub; using the installed {have}."));
        return Ok(have);
    }
    say(&format!("{want} is gone from GitHub; installing the latest Horizon instead..."));
    let before: Vec<_> = fs::read_dir(&versions).into_iter().flatten().flatten().map(|e| e.file_name()).collect();
    if run(&launcher_exe(), &["install", "latest"], &dir, say) != 0 {
        return Err("Installing Horizon in the Horizon launcher failed (see the log).".into());
    }
    let all = installed_bases();
    all.iter()
        .find(|v| !before.iter().any(|b| b.to_str() == Some(v.as_str())))
        .or(all.first())
        .cloned()
        .ok_or_else(|| "No Horizon version is installed in the Horizon launcher.".into())
}

fn launcher_instance() -> Option<Value> {
    launcher_json(&["instance", "list", "--json"])?
        .as_array()?
        .iter()
        .find(|i| i["id"] == "horizon-vr")
        .cloned()
}

fn has_launcher_instance() -> bool {
    launcher_instance().is_some()
}

fn selected_version() -> Option<String> {
    launcher_json(&["settings", "--json"])?["selected_version"].as_str().map(str::to_owned)
}

/// A "Horizon VR" instance in the Horizon launcher: the game imported (`import-build`) on the
/// launcher version it was built on (horizon-base.txt), so it plays with the launcher's account.
fn add_launcher_instance(dest: &Path, say: Say) -> R<()> {
    let hz_exe = launcher_exe();
    let base = dest.join("horizon-base.txt");
    if !hz_exe.exists() || !base.exists() {
        return Ok(());
    }
    let hz = |a: &[&str], what: &str| -> R<()> {
        if run(&hz_exe, a, dest, say) != 0 { Err(format!("{what} failed (see the log).")) } else { Ok(()) }
    };
    let selected = selected_version();
    let base = resolve_base(fs::read_to_string(&base).unwrap_or_default().trim(), dest, say)?;
    say(&format!("Adding the Horizon VR instance to the Horizon launcher (on {base})..."));
    let mut old = None;
    let mut new = String::new();
    let result = (|| -> R<()> {
        let client = dest.join("horizon-client.exe").display().to_string();
        hz(&["import-build", &client, "--version", &base, "--trust-unsigned"], "Importing the game")?;
        // import-build selects the version it made (the same one again for the same game).
        new = selected_version().filter(|v| v.starts_with("imported-")).ok_or("the imported version was not found.")?;
        let vdir = launcher().join("versions").join(&new);
        fs::copy(dest.join("openxr_loader.dll"), vdir.join("openxr_loader.dll")).map_err(|e| e.to_string())?;
        hz(&["prepare", "--version", &new], "Preparing the game files")?;
        match launcher_instance() {
            Some(mine) => old = mine["version"].as_str().map(str::to_owned),
            None => hz(&["instance", "create", "Horizon VR"], "Making the instance")?,
        }
        hz(&["instance", "version", "horizon-vr", &new], "Choosing the version")?;
        hz(&["instance", "launch", "horizon-vr", "--env", "HORIZON_VR=1"], "Turning VR on")?;
        // Before each start: the update question (`--check`), as the desktop shortcut asks it.
        // `if exist`: once uninstalled, the instance still starts.
        let me = dest.join(EXE).display().to_string();
        let check = format!("if exist \"{me}\" \"{me}\" --check --from-launcher");
        hz(&["instance", "launch", "horizon-vr", "--pre-launch", &check], "Adding the update check")?;
        run(&hz_exe, &["instance", "icon", "horizon-vr", "ender_eye"], dest, say);
        Ok(())
    })();
    // import-build selects what it imports: the player's own choice comes back, none included
    // (the CLI can't set none, so the settings file gets it).
    match selected {
        Some(s) => drop(run(&hz_exe, &["settings", "selected-version", &s], dest, say)),
        None => {
            let file = launcher().join("settings.json");
            if let Ok(text) = fs::read_to_string(&file) {
                let mut v: Value = serde_json::from_str(&text).unwrap_or_default();
                if v.get("selected").and_then(Value::as_str).is_some_and(|s| s.starts_with("imported-")) {
                    v["selected"] = Value::Null;
                    let _ = fs::write(&file, serde_json::to_string_pretty(&v).unwrap_or(text));
                }
            }
        }
    }
    result?;
    // The version an update replaced; nothing else plays it.
    if let Some(old) = old.filter(|o| o.starts_with("imported-") && *o != new) {
        run(&hz_exe, &["remove", &old], dest, say);
    }
    Ok(())
}

/// Takes the Horizon VR instance out of the launcher (its folder goes to the Trash, worlds
/// recoverable) and the imported version it played.
fn remove_launcher_instance(dest: &Path, say: Say) -> R<bool> {
    let hz = launcher_exe();
    if !hz.exists() {
        return Ok(false);
    }
    let Some(mine) = launcher_instance() else { return Ok(false) };
    say("Removing the Horizon VR instance from the Horizon launcher...");
    let dir = existing(dest);
    if run(&hz, &["instance", "delete", "horizon-vr"], &dir, say) != 0 {
        return Err("deleting it failed (see the log).".into());
    }
    if let Some(v) = mine["version"].as_str().filter(|v| v.starts_with("imported-")) {
        run(&hz, &["remove", v], &dir, say);
    }
    Ok(true)
}

/// Before each start: Horizon's Zoom mod into the VR instance (its own game folder has none),
/// from the player's main Horizon folder, on (`mod.zoom`) unless they turned it off.
fn copy_zoom() {
    let hz_home = env_dir("APPDATA").join("horizon");
    let vr = hz_home.join(r"instances\horizon-vr");
    let zoom = hz_home.join(r"horizon-native-mods\zoom.hnmod");
    let mods = vr.join("horizon-native-mods");
    if vr.exists() && zoom.exists() && !mods.join("zoom.hnmod").exists() {
        let _ = fs::create_dir_all(&mods);
        if fs::copy(&zoom, mods.join("zoom.hnmod")).is_ok() {
            let list = vr.join("horizon-mods.txt");
            let text = fs::read_to_string(&list).unwrap_or_default();
            if !text.contains("mod.zoom:") {
                let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\r\n" };
                let _ = fs::write(&list, format!("{text}{sep}mod.zoom:true\r\n"));
            }
        }
    }
}

pub fn message(text: &str, title: &str, yes_no: bool) -> bool {
    let d = rfd::MessageDialog::new().set_title(title).set_description(text);
    if yes_no {
        d.set_level(rfd::MessageLevel::Info).set_buttons(rfd::MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes
    } else {
        d.set_buttons(rfd::MessageButtons::Ok).show();
        true
    }
}

/// `--check` (Play Horizon VR.bat, before the game starts): asks to update when a newer release
/// is out. The exit code 1 stops a launcher start (the old version was chosen for it).
fn check(dest: &str, from_launcher: bool) -> i32 {
    copy_zoom();
    let have = read_version(&full_path(dest).join("version.txt"));
    let Some(have) = have else { return 0 };
    let Ok(latest) = get_latest() else { return 0 };
    if latest.version <= have {
        return 0;
    }
    let text = format!("Horizon VR {} is out (you have {have}).\n\nYes: update now\nNo: later", latest.version);
    if !message(&text, "Horizon VR update", true) {
        return 0;
    }
    let print = |t: &str| println!("{t}");
    match update(dest, &find_minecraft().display().to_string(), &find_server_jar(), &print) {
        Ok(_) if from_launcher => {
            message(&format!("Horizon VR is updated to {}. Click Play again.", latest.version), "Horizon VR", false);
            1
        }
        Ok(_) => {
            message(&format!("Horizon VR is updated to {}.", latest.version), "Horizon VR", false);
            0
        }
        Err(e) => {
            message(
                &format!("Update failed: {e}\n\nThe game starts as it is; try Update in Horizon VR Installer later."),
                "Horizon VR",
                false,
            );
            0
        }
    }
}

#[derive(Default)]
struct Args {
    silent: bool,
    repair: bool,
    uninstall: bool,
    update: bool,
    check: bool,
    from_launcher: bool,
    install_dir: Option<String>,
    minecraft_dir: Option<String>,
    server_jar: Option<String>,
}

/// `-Silent`, `--silent` and `/silent` alike (the PowerShell installer's switches, any case).
fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        let key = arg.trim_start_matches(['-', '/']).replace('-', "").to_ascii_lowercase();
        match key.as_str() {
            "silent" => a.silent = true,
            "repair" => a.repair = true,
            "uninstall" => a.uninstall = true,
            "update" => a.update = true,
            "check" => a.check = true,
            "fromlauncher" => a.from_launcher = true,
            "installdir" => a.install_dir = it.next(),
            "minecraftdir" => a.minecraft_dir = it.next(),
            "serverjar" => a.server_jar = it.next(),
            _ => {}
        }
    }
    a
}

/// A window-less program shows nothing in a console; print into the one it was started from.
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: plain Win32 calls with no pointers.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    let a = parse_args();
    let dest = a.install_dir.clone().unwrap_or_else(|| default_install_dir().display().to_string());
    if a.check {
        attach_console();
        std::process::exit(check(&dest, a.from_launcher));
    }
    if a.silent {
        attach_console();
        let mc = a.minecraft_dir.clone().unwrap_or_else(|| find_minecraft().display().to_string());
        let jar = a.server_jar.clone().unwrap_or_else(find_server_jar);
        let print = |t: &str| println!("{t}");
        let r = if a.uninstall {
            uninstall(&dest, &print)
        } else if a.update {
            update(&dest, &mc, &jar, &print).map(drop)
        } else if a.repair {
            repair(&dest, &mc, &jar, &print)
        } else {
            install(&dest, &mc, &jar, &print, None)
        };
        if let Err(e) = r {
            println!("Failed: {e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    gui::run(dest);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        let v = |s| Version::parse(s).unwrap();
        assert!(v("0.3.10") > v("0.3.9"));
        assert!(v("v1.0.0") > v("0.9.9"));
        assert_eq!(v("0.3.2\r\n").to_string(), "0.3.2");
        assert!(Version::parse("x").is_none());
    }

    #[test]
    fn full_path_trims() {
        assert_eq!(full_path(r#"C:\a b\"#), PathBuf::from(r"C:\a b"));
        assert_eq!(full_path(r#""C:\a b\""#), PathBuf::from(r"C:\a b"));
        assert_eq!(full_path(r"C:\"), PathBuf::from(r"C:\"));
    }

    #[test]
    fn run_exit_code_and_lines() {
        let dir = env::temp_dir();
        assert_eq!(run(Path::new("cmd.exe"), &["/c", "exit 3"], &dir, &|_| {}), 3);
        let seen = std::cell::RefCell::new(Vec::new());
        run(
            Path::new("cmd.exe"),
            &["/c", "echo out1& echo err1 1>&2& ping -n 2 127.0.0.1 >nul& echo out2& echo err2 1>&2"],
            &dir,
            &|t| seen.borrow_mut().push(t.trim().to_owned()),
        );
        let mut s = seen.into_inner();
        s.sort();
        assert_eq!(s.join(","), "err1,err2,out1,out2");
    }

    /// One test: the env vars it sets are the whole process's.
    #[test]
    fn base_fallback_and_server_jar() {
        let work = env::temp_dir().join(format!("horizonvr-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join(r"versions\nightly-old")).unwrap();
        let exe = work.join("hz.cmd");
        fs::write(
            &exe,
            "@echo off\r\n\
             if \"%1\"==\"install\" if \"%2\"==\"latest\" ( mkdir \"%~dp0versions\\nightly-new\" & exit /b 0 )\r\n\
             if \"%1\"==\"install\" if \"%2\"==\"nightly-kept\" ( mkdir \"%~dp0versions\\nightly-kept\" & exit /b 0 )\r\n\
             if \"%1\"==\"install\" exit /b 1\r\n\
             if \"%1\"==\"versions\" ( if exist \"%~dp0versions\\nightly-new\" ( echo {\"installed\":[\"nightly-new\",\"imported-x\"]} ) else ( echo {\"installed\":[\"imported-x\",\"nightly-old\"]} ) )\r\n",
        )
        .unwrap();
        // SAFETY: this is the only test that touches these variables.
        unsafe {
            env::set_var("HORIZONVR_LAUNCHER", &exe);
            env::set_var("HORIZONVR_LAUNCHER_DATA", &work);
        }
        let quiet = |_: &str| {};
        assert_eq!(resolve_base("nightly-old", &work, &quiet).unwrap(), "nightly-old", "installed pin");
        assert_eq!(resolve_base("nightly-kept", &work, &quiet).unwrap(), "nightly-kept", "downloadable pin");
        fs::remove_dir(work.join(r"versions\nightly-kept")).unwrap();
        assert_eq!(resolve_base("nightly-gone", &work, &quiet).unwrap(), "nightly-old", "gone pin, installed one");
        fs::remove_dir(work.join(r"versions\nightly-old")).unwrap();
        assert_eq!(resolve_base("nightly-gone", &work, &quiet).unwrap(), "nightly-new", "none installed: latest");

        // Another version's jar in Downloads is not taken for the 26.3 server jar.
        let home = work.join("home");
        fs::create_dir_all(home.join("Downloads")).unwrap();
        fs::File::create(home.join(r"Downloads\server-1.21.4.jar")).unwrap().set_len(31 << 20).unwrap();
        *SERVER_SHA.lock().unwrap() = Some("0".repeat(40));
        // SAFETY: as above.
        unsafe { env::set_var("USERPROFILE", &home) };
        assert_eq!(find_server_jar(), "");
        let _ = fs::remove_dir_all(&work);
    }
}
