use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

const REPO: &str = "irixsoft/irixmail";
const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

#[cfg(target_arch = "x86_64")]
const TARGET: &str = "x86_64-unknown-linux-musl";
#[cfg(target_arch = "aarch64")]
const TARGET: &str = "aarch64-unknown-linux-musl";
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const TARGET: &str = "unsupported";

pub(crate) fn newer_release() -> Option<String> {
    let agent = agent();
    let tag = latest_tag(&agent).ok()?;
    let latest = parse_version(&tag)?;
    let running = parse_version(env!("CARGO_PKG_VERSION"))?;
    (latest > running).then_some(tag)
}

pub fn run(check_only: bool) -> Result<()> {
    let agent = agent();
    let current = env!("CARGO_PKG_VERSION");
    let tag = latest_tag(&agent)?;
    let latest = parse_version(&tag)
        .ok_or_else(|| anyhow!("could not parse the latest release tag {tag}"))?;
    let running = parse_version(current)
        .ok_or_else(|| anyhow!("could not parse the running version {current}"))?;
    if latest <= running {
        println!("irixmail {current} is up to date.");
        return Ok(());
    }
    println!("A newer release is available: {current} -> {tag}");
    if check_only {
        println!("Install it with: sudo irixmail update");
        return Ok(());
    }
    if !crate::ownership::running_as_root() {
        bail!("updating needs root; run: sudo irixmail update");
    }

    let url = format!("https://github.com/{REPO}/releases/download/{tag}/irixmail-{TARGET}");
    println!("Downloading {url}");
    let binary = fetch(&agent, &url)?;
    let checksum = fetch(&agent, &format!("{url}.sha256"))?;
    verify_sha256(&binary, &String::from_utf8_lossy(&checksum))?;

    let exe = std::env::current_exe().context("locating the running binary")?;
    replace_binary(&exe, &binary)?;
    println!("Installed irixmail {tag} at {}", exe.display());
    restart_service();
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .into()
}

fn latest_tag(agent: &ureq::Agent) -> Result<String> {
    use ureq::ResponseExt;
    let url = format!("https://github.com/{REPO}/releases/latest");
    let response = agent
        .get(&url)
        .call()
        .with_context(|| format!("checking the latest release at {url}"))?;
    let landed = response.get_uri().to_string();
    version_from_release_url(&landed)
        .ok_or_else(|| anyhow!("no published release found (landed on {landed})"))
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut response = agent
        .get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))
}

fn version_from_release_url(url: &str) -> Option<String> {
    let tag = url.rsplit_once("/releases/tag/")?.1;
    let tag = tag.split(['?', '#']).next()?;
    (!tag.is_empty()).then(|| tag.to_string())
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn verify_sha256(data: &[u8], checksum_text: &str) -> Result<()> {
    let expected = checksum_text.split_whitespace().next().unwrap_or("");
    if expected.is_empty() {
        bail!("the published checksum was empty");
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
    let actual: String = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("checksum verification failed");
    }
    Ok(())
}

fn replace_binary(exe: &Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("the running binary has no parent directory"))?;
    let temp = dir.join(".irixmail.update.tmp");
    let install = || -> Result<()> {
        std::fs::write(&temp, data)
            .with_context(|| format!("writing the new binary to {}", temp.display()))?;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))
            .context("marking the new binary executable")?;
        std::fs::rename(&temp, exe)
            .with_context(|| format!("installing the new binary at {}", exe.display()))?;
        Ok(())
    };
    install().inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

fn restart_service() {
    let active = Command::new("systemctl")
        .args(["is-active", "--quiet", "irixmail"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !active {
        println!("Restart the server to run the new version.");
        return;
    }
    match Command::new("systemctl")
        .args(["restart", "irixmail"])
        .status()
    {
        Ok(status) if status.success() => println!("Service irixmail restarted."),
        _ => println!("Could not restart the service; run: sudo systemctl restart irixmail"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_tag_is_read_from_the_redirected_url() {
        assert_eq!(
            version_from_release_url("https://github.com/irixsoft/irixmail/releases/tag/v0.0.3"),
            Some("v0.0.3".into())
        );
        assert_eq!(
            version_from_release_url("https://github.com/irixsoft/irixmail/releases"),
            None
        );
        assert_eq!(
            version_from_release_url("https://github.com/irixsoft/irixmail/releases/tag/"),
            None
        );
    }

    #[test]
    fn versions_parse_and_compare_numerically() {
        assert_eq!(parse_version("0.0.3"), Some((0, 0, 3)));
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert!(parse_version("v0.10.2") > parse_version("v0.9.9"));
        assert_eq!(parse_version("abc"), None);
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn a_checksum_mismatch_is_an_error_and_a_match_is_not() {
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  irixmail";
        assert!(verify_sha256(b"abc", good).is_ok());
        assert!(verify_sha256(b"abd", good).is_err());
        assert!(verify_sha256(b"abc", "").is_err());
    }
}
