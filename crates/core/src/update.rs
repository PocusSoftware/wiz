//! Looking up the newest Wiz release on GitHub and fetching the file for this platform.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const REPO: &str = "PocusSoftware/wiz";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const NOTES_MAX: usize = 4000;

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub asset: Asset,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

pub fn asset_name() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("Wiz-macOS-arm64.zip")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("Wiz-windows-x64.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("wiz-linux-x86_64.tar.gz")
    } else {
        None
    }
}

fn version_parts(v: &str) -> Option<[u64; 3]> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = it.next()?.parse().ok()?;
    }
    it.next().is_none().then_some(out)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!(
        (version_parts(candidate), version_parts(current)),
        (Some(a), Some(b)) if a > b
    )
}

pub fn parse_release(json: &str, asset: &str) -> Result<Option<Release>> {
    let r: GhRelease = serde_json::from_str(json).map_err(|e| Error::Update(e.to_string()))?;
    if r.draft || r.prerelease || version_parts(&r.tag_name).is_none() {
        return Ok(None);
    }
    let Some(a) = r.assets.into_iter().find(|a| a.name == asset) else {
        return Ok(None);
    };
    let mut notes = r.body.unwrap_or_default();
    if notes.len() > NOTES_MAX {
        let mut end = NOTES_MAX;
        while !notes.is_char_boundary(end) {
            end -= 1;
        }
        notes.truncate(end);
        notes.push_str("...");
    }
    Ok(Some(Release {
        version: r.tag_name.trim_start_matches('v').to_string(),
        notes,
        page: r.html_url,
        asset: Asset {
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
            sha256: a
                .digest
                .and_then(|d| d.strip_prefix("sha256:").map(str::to_lowercase)),
        },
    }))
}

pub fn newer_release() -> Result<Option<Release>> {
    let Some(asset) = asset_name() else {
        return Ok(None);
    };
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let out = curl(&[
        "-fsSL".as_ref(),
        "--max-time".as_ref(),
        "20".as_ref(),
        "-H".as_ref(),
        "Accept: application/vnd.github+json".as_ref(),
        "-H".as_ref(),
        "User-Agent: Wiz".as_ref(),
        url.as_ref(),
    ])?;
    let json = String::from_utf8(out).map_err(|e| Error::Update(e.to_string()))?;
    Ok(parse_release(&json, asset)?.filter(|r| is_newer(&r.version, CURRENT)))
}

pub fn download(asset: &Asset, dest: &Path) -> Result<()> {
    curl(&[
        "-fsSL".as_ref(),
        "--max-time".as_ref(),
        "900".as_ref(),
        "-o".as_ref(),
        dest.as_os_str(),
        asset.url.as_ref(),
    ])?;
    verify(dest, asset)
}

fn verify(path: &Path, asset: &Asset) -> Result<()> {
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 != asset.size {
        return Err(Error::Update("the download is incomplete".into()));
    }
    if let Some(want) = &asset.sha256 {
        let got: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if &got != want {
            return Err(Error::Update(
                "the download does not match its checksum".into(),
            ));
        }
    }
    Ok(())
}

fn curl_bin() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/usr/bin/curl")
    } else if cfg!(windows) {
        std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join(r"System32\curl.exe"))
            .unwrap_or_else(|| PathBuf::from("curl.exe"))
    } else {
        PathBuf::from("curl")
    }
}

fn curl(args: &[&OsStr]) -> Result<Vec<u8>> {
    let out = crate::process::command(curl_bin()).args(args).output()?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(Error::Update(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{
        "tag_name": "v0.26.0",
        "body": "Faster search",
        "html_url": "https://github.com/PocusSoftware/wiz/releases/tag/v0.26.0",
        "draft": false,
        "prerelease": false,
        "assets": [
            {"name": "Wiz-windows-x64.exe", "browser_download_url": "https://e/w", "size": 3, "digest": "sha256:ABC"},
            {"name": "Wiz-macOS-arm64.zip", "browser_download_url": "https://e/m", "size": 5}
        ]
    }"#;

    #[test]
    fn compares_versions_by_number() {
        assert!(is_newer("v0.26.0", "0.25.3"));
        assert!(is_newer("0.25.10", "0.25.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.25.3", "0.25.3"));
        assert!(!is_newer("0.25.2", "0.25.3"));
        assert!(!is_newer("nightly", "0.25.3"));
        assert!(!is_newer("0.26", "0.25.3"));
        assert!(!is_newer("0.26.0-beta.1", "0.25.3"));
    }

    #[test]
    fn picks_this_platforms_file() {
        let r = parse_release(JSON, "Wiz-windows-x64.exe").unwrap().unwrap();
        assert_eq!(r.version, "0.26.0");
        assert_eq!(r.notes, "Faster search");
        assert_eq!(r.asset.url, "https://e/w");
        assert_eq!(r.asset.sha256.as_deref(), Some("abc"));
        let m = parse_release(JSON, "Wiz-macOS-arm64.zip").unwrap().unwrap();
        assert_eq!(m.asset.sha256, None);
        assert_eq!(
            parse_release(JSON, "wiz-linux-x86_64.tar.gz").unwrap(),
            None
        );
    }

    #[test]
    fn skips_drafts_prereleases_and_odd_tags() {
        for (from, to) in [
            (r#""draft": false"#, r#""draft": true"#),
            (r#""prerelease": false"#, r#""prerelease": true"#),
            ("v0.26.0", "latest"),
        ] {
            let json = JSON.replace(from, to);
            assert_eq!(parse_release(&json, "Wiz-windows-x64.exe").unwrap(), None);
        }
    }

    #[test]
    fn long_notes_are_cut() {
        let json = JSON.replace("Faster search", &"é".repeat(NOTES_MAX));
        let r = parse_release(&json, "Wiz-windows-x64.exe")
            .unwrap()
            .unwrap();
        assert!(r.notes.len() <= NOTES_MAX + 3 && r.notes.ends_with("..."));
    }

    #[test]
    fn downloads_are_checked_by_size_and_hash() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("w");
        std::fs::write(&f, b"abc").unwrap();
        let asset = |size, sha: Option<&str>| Asset {
            name: "w".into(),
            url: String::new(),
            size,
            sha256: sha.map(String::from),
        };
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(&f, &asset(3, Some(good))).is_ok());
        assert!(verify(&f, &asset(3, None)).is_ok());
        assert!(verify(&f, &asset(4, Some(good))).is_err());
        assert!(verify(&f, &asset(3, Some(&"0".repeat(64)))).is_err());
    }
}
