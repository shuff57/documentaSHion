//! Self-update from the source checkout: fetch GitHub, and if it is ahead, pull, rebuild,
//! reinstall and restart. Needs git and cargo on the machine, which a source install has.
use crate::services::process_for;
use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::AtomicBool,
    time::Duration,
};

/// The commit this binary was built from.
pub const BUILT: &str = env!("DOCUMENTASHION_COMMIT");

pub struct Status {
    /// Subject lines of the commits not yet installed, newest first.
    pub changes: Vec<String>,
    pub behind: bool,
}

/// The checkout to update from: `DOCUMENTASHION_SRC`, else ~/Documents/GitHub/documentaSHion.
pub fn source() -> PathBuf {
    std::env::var_os("DOCUMENTASHION_SRC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs_home().join("Documents/GitHub/documentaSHion")
        })
}
fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}
fn git(dir: &PathBuf, args: &[&str], cancel: &AtomicBool) -> Result<String, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0");
    process_for(c, cancel, Duration::from_secs(120)).map(|s| s.trim().to_owned())
}
fn checkout() -> Result<PathBuf, String> {
    let dir = source();
    if dir.join(".git").exists() {
        Ok(dir)
    } else {
        Err(format!("No source checkout at {}. Set DOCUMENTASHION_SRC to where you cloned it.", dir.display()))
    }
}

pub fn check(cancel: &AtomicBool) -> Result<Status, String> {
    let dir = checkout()?;
    git(&dir, &["fetch", "origin", "main"], cancel)?;
    let latest = git(&dir, &["rev-parse", "origin/main"], cancel)?;
    let behind = latest != BUILT;
    let changes = if behind {
        git(&dir, &["log", "--format=%s", &format!("{BUILT}..origin/main")], cancel)
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    } else {
        vec![]
    };
    Ok(Status { changes, behind })
}

/// Pulls, builds in release mode, installs, and starts the new copy. The caller then exits.
pub fn apply(cancel: &AtomicBool) -> Result<(), String> {
    let dir = checkout()?;
    let run = |program: &str, args: &[&str], limit: u64| -> Result<String, String> {
        let mut c = Command::new(program);
        c.current_dir(&dir).args(args).env("GIT_TERMINAL_PROMPT", "0");
        // A desktop launcher often lacks cargo on its PATH.
        let path = std::env::var("PATH").unwrap_or_default();
        c.env("PATH", format!("{}/.cargo/bin:{path}", dirs_home().display()));
        process_for(c, cancel, Duration::from_secs(limit)).map_err(|e| format!("{program} {}: {e}", args.join(" ")))
    };
    run("git", &["pull", "--ff-only", "origin", "main"], 120)?;
    run("cargo", &["build", "--release", "--locked"], 1800)?;
    run("bash", &["install.sh"], 60)?;
    Command::new(dirs_home().join(".local/bin/documentashion"))
        .spawn()
        .map_err(|e| format!("Installed, but couldn't restart: {e}"))?;
    Ok(())
}
