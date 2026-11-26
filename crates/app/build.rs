//! Embeds the git commit (`WIZ_COMMIT`) so builds can be told apart beyond the version.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    // Rebuild when HEAD moves. Only existing paths are watched: a missing one would make
    // Cargo rerun this script (and relink the app) on every build.
    // `--git-path` resolves each file correctly in worktrees too.
    let mut watch = vec!["HEAD".to_string(), "packed-refs".to_string()];
    watch.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for name in watch {
        if let Some(p) = git(&["rev-parse", "--path-format=absolute", "--git-path", &name])
            && Path::new(&p).exists()
        {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=WIZ_COMMIT={commit}");

    let target = |key: &str| std::env::var(key).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        println!("cargo:rerun-if-changed=assets/windows/wiz.ico");
        embed_resource::compile("assets/windows/wiz.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the icon resource compiles with the Windows SDK's rc.exe");
    }
}
