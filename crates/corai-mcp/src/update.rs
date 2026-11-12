//! Release check against GitHub (port of the Node `checkForUpdates`). Installing updates of the
//! native binaries arrives with the release pipeline.

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

fn latest_release(repo: &str) -> Result<Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "ed-corai-update-check");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let mut res = req.call().map_err(|e| e.to_string())?;
    match res.status().as_u16() {
        404 => return Err(format!("No published releases found for {repo}.")),
        200..=299 => {}
        s => return Err(format!("GitHub answered {s} for {repo}.")),
    }
    let r: Value =
        serde_json::from_str(&res.body_mut().read_to_string().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let tag = r["tag_name"].as_str().unwrap_or_default().to_string();
    let notes = r["body"].as_str().unwrap_or_default();
    let notes = if notes.chars().count() > 3000 {
        format!("{}\n…", notes.chars().take(3000).collect::<String>())
    } else {
        notes.to_string()
    };
    Ok(json!({
        "tag": tag,
        "version": tag.trim_start_matches(['v', 'V']),
        "name": r["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&tag),
        "url": r["html_url"],
        "publishedAt": r["published_at"],
        "notes": notes,
    }))
}

/// Installed version, latest release and whether an update is available.
pub fn check_for_updates() -> Value {
    let current = build_info();
    let installed = env!("CARGO_PKG_VERSION");
    let repo = repo();
    match latest_release(&repo) {
        Err(e) => json!({
            "current": current, "repo": repo, "updateAvailable": false, "canAutoInstall": false,
            "message": format!("Could not check for updates: {e}"),
        }),
        Ok(latest) => {
            let version = latest["version"].as_str().unwrap_or_default();
            let available = compare_versions(version, installed) == std::cmp::Ordering::Greater;
            let message = if available {
                format!(
                    "ED Corai {version} is available (installed {installed}). Download it from {}.",
                    latest["url"].as_str().unwrap_or_default()
                )
            } else {
                format!(
                    "ED Corai {installed} is up to date (latest release {}).",
                    latest["tag"].as_str().unwrap_or_default()
                )
            };
            json!({
                "current": current, "repo": repo, "latest": latest, "updateAvailable": available,
                "canAutoInstall": false, "message": message,
            })
        }
    }
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
