use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{fs, process::Command};

use crate::state::remove;

pub fn execute_command(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn {cmd}"))?;
    if !status.success() {
        bail!("{cmd} failed with {args:?}");
    }
    Ok(())
}

pub fn get_vhost_vchild(name: &str) -> (String, String) {
    let mut hasher = Sha256::new();
    hasher.update(name);
    let truncated_hash = &hex::encode(hasher.finalize())[..12];
    (
        format!("vh-{truncated_hash}"),
        format!("vc-{truncated_hash}"),
    )
}

pub fn cleanup(name: &str) {
    let (vhost, _) = get_vhost_vchild(name);

    let _ = Command::new("ip")
        .args(["link", "delete", &vhost])
        .stderr(std::process::Stdio::null())
        .status();

    let _ = fs::remove_dir(format!("/sys/fs/cgroup/rcr/{name}"));
    // Delete upper of overlayfs
    let _ = fs::remove_dir_all(format!("/run/rcr/{name}"));

    remove(name);
}
