//! Release check against GitHub (port of the Node `checkForUpdates`) and self-update: download this
//! platform's release zip, verify it against the release's SHA256SUMS.txt and replace the programs
//! (the command-line program and the window) in the folder of the running one.

use std::time::Duration;

use serde_json::{Value, json};

/// Version of this build and of the Coriolis code and data compiled into it.
pub fn build_info() -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("CORAI_GIT_COMMIT"),
        "coriolis": { "commit": option_env!("CORAI_CORIOLIS_COMMIT"), "date": option_env!("CORAI_CORIOLIS_DATE") },
        "coriolisData": { "commit": option_env!("CORAI_CORIOLIS_DATA_COMMIT"), "date": option_env!("CORAI_CORIOLIS_DATA_DATE") },
    })
}

/// Compares two semantic versions (`v` prefix allowed); pre-releases sort before their release.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let parse = |v: &str| {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let v = v.split('+').next().unwrap_or_default();
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let nums: Vec<u64> = core.split('.').map(|x| x.parse().unwrap_or(0)).collect();
        (nums, pre.to_string())
    };
    let ((x, xp), (y, yp)) = (parse(a), parse(b));
    for i in 0..x.len().max(y.len()).max(3) {
        match x.get(i).unwrap_or(&0).cmp(y.get(i).unwrap_or(&0)) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    match (xp.is_empty(), yp.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => natural_cmp(&xp, &yp),
    }
}

/// Compares strings with digit runs as numbers (`beta.10` after `beta.2`).
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let mut text = String::new();
        let mut digits = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
            } else {
                if !digits.is_empty() {
                    out.push((std::mem::take(&mut text), digits.parse().unwrap_or(0)));
                    digits.clear();
                }
                text.push(c);
            }
        }
        out.push((text, digits.parse().unwrap_or(0)));
        out
    };
    split(a).cmp(&split(b))
}

/// `owner/name` from a GitHub URL or a bare slug.
pub fn parse_github_slug(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    let tail = s
        .rsplit_once("github.com")
        .map_or(s, |(_, t)| t.trim_start_matches([':', '/']));
    let mut parts = tail.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(o), Some(n), None)
            if !o.is_empty() && !n.is_empty() && (s.contains("github.com") || !s.contains(':')) =>
        {
            Some(format!("{o}/{n}"))
        }
        _ => None,
    }
}

/// The repository to check: `CORAI_REPO`, else the project's own.
pub fn repo() -> String {
    std::env::var("CORAI_REPO")
        .ok()
        .and_then(|r| parse_github_slug(&r))
        .unwrap_or_else(|| "kay-dutch/ed-corai".to_string())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

fn github(url: &str) -> Result<Value, String> {
    let mut req = agent(Duration::from_secs(8))
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "ed-corai-update-check");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let mut res = req.call().map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let body = res.body_mut().read_to_string().map_err(|e| e.to_string())?;
    if !(200..300).contains(&status) {
        return Err(status.to_string());
    }
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// The raw GitHub release: the latest, or the one with `tag`.
fn fetch_release(repo: &str, tag: Option<&str>) -> Result<Value, String> {
    let url = match tag {
        Some(t) => format!("https://api.github.com/repos/{repo}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    github(&url).map_err(|status| match (status.as_str(), tag) {
        ("404", None) => format!("No published releases found for {repo}."),
        ("404", Some(t)) => format!("Release {t} not found in {repo}."),
        (s, _) if s.chars().all(|c| c.is_ascii_digit()) => {
            format!("GitHub answered {s} for {repo}.")
        }
        (other, _) => other.to_string(),
    })
}

fn release_summary(r: &Value) -> Value {
    let tag = r["tag_name"].as_str().unwrap_or_default().to_string();
    let notes = r["body"].as_str().unwrap_or_default();
    let notes = if notes.chars().count() > 3000 {
        format!("{}\n…", notes.chars().take(3000).collect::<String>())
    } else {
        notes.to_string()
    };
    json!({
        "tag": tag,
        "version": tag.trim_start_matches(['v', 'V']),
        "name": r["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&tag),
        "url": r["html_url"],
        "publishedAt": r["published_at"],
        "notes": notes,
    })
}

/// The release asset with this platform's program, e.g. `ed-corai-0.2.0-windows-x64.zip`.
pub fn asset_name(version: &str) -> Option<String> {
    if cfg!(target_os = "windows") {
        Some(format!("ed-corai-{version}-windows-x64.zip"))
    } else if cfg!(target_os = "macos") {
        Some(format!("ed-corai-{version}-macos-universal.zip"))
    } else {
        None
    }
}

/// File name of the command-line program (the MCP server) inside the release zip.
pub fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ed-corai-cli.exe"
    } else {
        "ed-corai-cli"
    }
}

/// File name of the desktop window inside the release zip (from v0.3.0).
pub fn window_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ed-corai.exe"
    } else {
        "ed-corai"
    }
}

fn asset_url<'a>(release: &'a Value, name: &str) -> Option<&'a str> {
    release["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"] == name)
        .and_then(|a| a["browser_download_url"].as_str())
}

/// Installed version, latest release and whether an update is available.
pub fn check_for_updates() -> Value {
    let current = build_info();
    let installed = env!("CARGO_PKG_VERSION");
    let repo = repo();
    match fetch_release(&repo, None) {
        Err(e) => json!({
            "current": current, "repo": repo, "updateAvailable": false, "canAutoInstall": false,
            "message": format!("Could not check for updates: {e}"),
        }),
        Ok(raw) => {
            let latest = release_summary(&raw);
            let version = latest["version"].as_str().unwrap_or_default().to_string();
            let available = compare_versions(&version, installed) == std::cmp::Ordering::Greater;
            let installable = asset_name(&version).is_some_and(|a| asset_url(&raw, &a).is_some())
                && asset_url(&raw, CHECKSUMS).is_some();
            let message = match (available, installable) {
                (false, _) => format!(
                    "ED Corai {installed} is up to date (latest release {}).",
                    latest["tag"].as_str().unwrap_or_default()
                ),
                (true, true) => format!(
                    "ED Corai {version} is available (installed {installed}). It can be installed automatically; restart the MCP server afterwards."
                ),
                (true, false) => format!(
                    "ED Corai {version} is available (installed {installed}). Download it from {}.",
                    latest["url"].as_str().unwrap_or_default()
                ),
            };
            json!({
                "current": current, "repo": repo, "latest": latest, "updateAvailable": available,
                "canAutoInstall": available && installable, "message": message,
            })
        }
    }
}

const CHECKSUMS: &str = "SHA256SUMS.txt";
/// Upper bound on a download, far above the real size, against runaway responses.
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

fn download(url: &str) -> Result<Vec<u8>, String> {
    let mut res = agent(Duration::from_secs(300))
        .get(url)
        .header("User-Agent", "ed-corai-updater")
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("Download failed: HTTP {}", res.status().as_u16()));
    }
    res.body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| format!("Download failed: {e}"))
}

/// The hash `SHA256SUMS.txt` lists for `name` (`<hex>  <name>` per line).
pub fn expected_checksum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_lowercase())
    })
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The program file from a release zip (matched by file name, in any folder).
pub fn extract_binary(zip_bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| format!("Not a valid release zip: {e}"))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let is_it = file.name().is_ok_and(|n| {
            std::path::Path::new(n.as_ref())
                .file_name()
                .is_some_and(|f| f == name)
        });
        if is_it {
            let mut out = Vec::new();
            file.read_to_end(&mut out).map_err(|e| e.to_string())?;
            return Ok(out);
        }
    }
    Err(format!("The release zip does not contain {name}."))
}

/// Where the previous program goes on Windows (a running program cannot be overwritten there, but
/// it can be renamed).
fn previous_path(exe: &std::path::Path) -> std::path::PathBuf {
    exe.with_extension("old")
}

/// Puts `program` in place of `exe`: written next to it first, then swapped in.
pub fn replace_executable(exe: &std::path::Path, program: &[u8]) -> std::io::Result<()> {
    let dir = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
    let staged = dir.join(format!(
        ".{}.new",
        exe.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("ed-corai-cli")
    ));
    std::fs::write(&staged, program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    if cfg!(windows) && exe.exists() {
        let old = previous_path(exe);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old)?;
        if let Err(e) = std::fs::rename(&staged, exe) {
            let _ = std::fs::rename(&old, exe); // put the working program back
            return Err(e);
        }
    } else {
        std::fs::rename(&staged, exe)?;
    }
    Ok(())
}

/// Removes the previous programs left behind by an update on Windows (one still running stays).
pub fn cleanup_previous() {
    if let Some(dir) = program_dir() {
        for name in [binary_name(), window_name()] {
            let _ = std::fs::remove_file(previous_path(&dir.join(name)));
        }
    }
}

/// The folder of the running program, where both programs live.
pub fn program_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(std::path::Path::to_path_buf)
}

/// Installs the programs from a release zip into `dir`: the command-line program (must be in the
/// zip) and the window (when the zip has one). Both are read from the zip before either is
/// replaced. Returns the replaced paths.
///
/// # Errors
/// The zip lacks the command-line program or a file cannot be replaced.
pub fn install_programs(
    zip_bytes: &[u8],
    dir: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let cli = extract_binary(zip_bytes, binary_name())?;
    let window = extract_binary(zip_bytes, window_name()).ok();
    let mut done = Vec::new();
    for (name, program) in [(binary_name(), Some(cli)), (window_name(), window)] {
        let Some(program) = program else { continue };
        let path = dir.join(name);
        replace_executable(&path, &program)
            .map_err(|e| format!("Could not replace {}: {e}", path.display()))?;
        done.push(path);
    }
    Ok(done)
}

/// Downloads this platform's programs from a release (default: the latest), verifies them against
/// the release checksums and replaces them next to the running program. Running copies keep the
/// old version until restarted.
///
/// # Errors
/// No release or asset for this platform, a failed download, a checksum mismatch, or the program
/// file cannot be replaced.
pub fn install_update(tag: Option<&str>) -> Result<Value, String> {
    let repo = repo();
    let release = fetch_release(&repo, tag)?;
    let summary = release_summary(&release);
    let version = summary["version"].as_str().unwrap_or_default().to_string();
    let asset = asset_name(&version)
        .ok_or("Automatic updates are available for Windows and macOS only.")?;
    let asset_link =
        asset_url(&release, &asset).ok_or_else(|| format!("Release {version} has no {asset}."))?;
    let sums_link = asset_url(&release, CHECKSUMS)
        .ok_or_else(|| format!("Release {version} has no {CHECKSUMS}."))?;
    let zip_bytes = download(asset_link)?;
    let sums = String::from_utf8_lossy(&download(sums_link)?).into_owned();
    let want = expected_checksum(&sums, &asset)
        .ok_or_else(|| format!("{CHECKSUMS} does not list {asset}."))?;
    if sha256_hex(&zip_bytes) != want {
        return Err(format!(
            "The downloaded {asset} does not match its published checksum; nothing was changed."
        ));
    }
    let dir = program_dir().ok_or("Cannot locate the running program.")?;
    let replaced = install_programs(&zip_bytes, &dir)?;
    Ok(json!({
        "installed": summary["tag"],
        "from": env!("CARGO_PKG_VERSION"),
        "paths": replaced.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "restartRequired": true,
        "message": format!("ED Corai {version} is installed. Restart the MCP server (Claude Code: /mcp; Claude Desktop: restart the app) and the ED Corai window if it is open."),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn compares_semantic_versions() {
        assert_eq!(compare_versions("0.2.0", "0.1.9"), Greater);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("v1.0.0", "1.0.0"), Equal);
        assert_eq!(compare_versions("1.0.0-rc.1", "1.0.0"), Less);
        assert_eq!(compare_versions("1.0.0-beta.10", "1.0.0-beta.2"), Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Equal);
    }

    #[test]
    fn finds_checksums_by_file_name() {
        let sums =
            "ABC123  ed-corai-0.2.0-windows-x64.zip\nfff  ed-corai-0.2.0-macos-universal.zip\n";
        assert_eq!(
            expected_checksum(sums, "ed-corai-0.2.0-windows-x64.zip").as_deref(),
            Some("abc123")
        );
        assert_eq!(expected_checksum(sums, "missing.zip"), None);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn extracts_the_program_and_swaps_it_in() {
        use std::io::Write;
        let zip_of = |files: &[(&str, &[u8])]| {
            let mut buf = std::io::Cursor::new(Vec::new());
            let mut z = zip::ZipWriter::new(&mut buf);
            for (name, bytes) in files {
                z.start_file(*name, zip::write::SimpleFileOptions::default())
                    .expect("zip");
                z.write_all(bytes).expect("zip");
            }
            z.finish().expect("zip");
            buf.into_inner()
        };
        let both = zip_of(&[
            ("README.md", b"readme"),
            (&format!("ed-corai/{}", binary_name()), b"new cli"),
            (window_name(), b"new window"),
        ]);
        assert_eq!(
            extract_binary(&both, binary_name()).expect("found"),
            b"new cli"
        );
        assert!(extract_binary(&both, "nope").is_err());

        let dir = std::env::temp_dir().join(format!("corai-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let cli = dir.join(binary_name());
        std::fs::write(&cli, b"old cli").expect("write");
        // Before v0.3.0 the folder had no window: the update adds it.
        let replaced = install_programs(&both, &dir).expect("installed");
        assert_eq!(replaced.len(), 2);
        assert_eq!(std::fs::read(&cli).expect("read"), b"new cli");
        assert_eq!(
            std::fs::read(dir.join(window_name())).expect("read"),
            b"new window"
        );

        // A zip without the window (v0.2.0) still updates the command-line program.
        let cli_only = zip_of(&[(binary_name(), b"newer cli")]);
        assert_eq!(
            install_programs(&cli_only, &dir).expect("installed").len(),
            1
        );
        assert_eq!(std::fs::read(&cli).expect("read"), b"newer cli");
        // Without the command-line program nothing is touched.
        let window_only = zip_of(&[(window_name(), b"x")]);
        assert!(install_programs(&window_only, &dir).is_err());
        assert_eq!(
            std::fs::read(dir.join(window_name())).expect("read"),
            b"new window"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_github_repositories() {
        for url in [
            "https://github.com/kay-dutch/ed-corai.git",
            "https://github.com/kay-dutch/ed-corai",
            "git@github.com:kay-dutch/ed-corai.git",
            "kay-dutch/ed-corai",
        ] {
            assert_eq!(
                parse_github_slug(url).as_deref(),
                Some("kay-dutch/ed-corai"),
                "{url}"
            );
        }
        assert_eq!(parse_github_slug("https://gitlab.com/x/y.git"), None);
    }
}
