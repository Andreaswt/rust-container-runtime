use crate::state::remove;
use std::{fs, process::Command};

pub fn execute_command(cmd: &str, args: &[&str]) {
    let status = Command::new(cmd).args(args).status();
    if !status.unwrap().success() {
        panic!("{cmd} failed with {args:?}");
    }
}

pub fn get_vhost_vchild(name: &str) -> (String, String) {
    return (format!("vh-{name}"), format!("vc-{name}"));
}

pub fn cleanup(name: &str) {
    let (vhost, _) = get_vhost_vchild(name);

    let _ = Command::new("ip")
        .args(["link", "delete", &vhost])
        .stderr(std::process::Stdio::null())
        .status();

    let _ = fs::remove_dir(format!("/sys/fs/cgroup/rcr/{name}"));

    remove(name);
}
