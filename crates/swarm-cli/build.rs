//! Capture source provenance at build time, never from the user's runtime
//! working directory. A dirty checkout is not exact-commit evidence.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

fn main() {
    for path in [
        "src",
        "build.rs",
        "Cargo.toml",
        "../../Cargo.lock",
        "../swarm-core",
        "../swarm-storage",
        "../swarm-protocol",
        "../swarm-network",
        "../swarm-consensus",
        "../swarm-ipc",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for path in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    let in_repo = git(&["rev-parse", "--show-prefix"]).as_deref() == Some("crates/swarm-cli/");
    let commit = in_repo
        .then(|| git(&["rev-parse", "--verify", "HEAD"]))
        .flatten()
        .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let tree = match in_repo.then(|| git(&["status", "--porcelain", "--untracked-files=normal"])).flatten() {
        Some(status) if status.is_empty() && commit.is_some() => "clean",
        Some(_) => "modified",
        None => "unknown",
    };
    println!("cargo:rustc-env=SWARMCRAFT_CLI_SUPPORT_COMMIT={}", commit.unwrap_or_default());
    println!("cargo:rustc-env=SWARMCRAFT_CLI_SUPPORT_TREE={tree}");
}
