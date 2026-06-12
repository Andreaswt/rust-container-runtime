use anyhow::{Context, Result};
use std::env::set_var;
use std::fs::{self, read_to_string};

use crate::utils::{execute_command, get_vhost_vchild};
use caps::{CapSet, Capability};

use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};
use nix::libc;
use nix::mount::{MntFlags, MsFlags, mount, umount2};
use nix::sched::{CloneFlags, unshare};
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, chdir, execvp, fork, pivot_root, read, sethostname, write};
use std::ffi::CString;
use std::os::fd::{FromRawFd, OwnedFd};

pub fn run_container(
    name: &str,
    child_ip: &str,
    lower: &str,
    netns_ready_w: i32,
    net_configured_r: i32,
    command: Vec<String>,
) -> Result<()> {
    unshare(
        CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWNET,
    )
    .context("unshare failed")?;

    let netns_ready_w = unsafe { OwnedFd::from_raw_fd(netns_ready_w) };
    write(&netns_ready_w, &[1u8]).context("signal netns_ready failed")?;

    match unsafe { fork() }.context("fork failed")? {
        ForkResult::Child => {
            let net_configured_r = unsafe { OwnedFd::from_raw_fd(net_configured_r) };
            let mut buf = [0u8; 1];
            read(&net_configured_r, &mut buf).context("wait net_configured failed")?;

            mount(
                None::<&str>,
                "/",
                None::<&str>,
                MsFlags::MS_REC | MsFlags::MS_PRIVATE,
                None::<&str>,
            )
            .context("make / private mounts failed")?;

            setup_container_networking(&name, &child_ip)?;

            sethostname(&name).context("sethostname failed")?;

            // Join cgroup, then chroot removes privileges to unjoin
            std::fs::write(format!("/sys/fs/cgroup/rcr/{name}/cgroup.procs"), "0")
                .context("self-join cgroup failed")?;

            let merged = setup_overlay(name, lower)?;
            setup_dns(&merged);
            pivot_root_util(&merged)?;

            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::empty(),
                None::<&str>,
            )
            .context("proc mount failed")?;

            drop_capabilities();
            apply_seccomp()?;

            if let Ok(envs) = read_to_string("/etc/rcr-env") {
                for line in envs.lines() {
                    if let Some((key, value)) = line.split_once("=") {
                        unsafe {
                            set_var(key, value);
                        }
                    }
                }
            }

            let args: Vec<CString> = command
                .iter()
                .map(|s| CString::new(s.as_str()).context("cstring creation failed"))
                .collect::<Result<Vec<_>>>()?;

            execvp(&args[0], &args)
                .with_context(|| format!("failed to execute '{}'", &command[0]))?;
        }
        ForkResult::Parent { child } => {
            waitpid(child, None).context("waitpid failed")?;
        }
    }

    Ok(())
}

fn setup_dns(layer: &str) {
    let target = format!("{layer}/etc/resolv.conf");
    let _ = fs::create_dir_all(format!("{layer}/etc"));

    let host = fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let content = if host.contains("127.0.0") {
        "nameserver 1.1.1.1\n".to_string()
    } else {
        host
    };
    let _ = fs::write(&target, content);
}

fn setup_container_networking(name: &str, child_ip: &str) -> Result<()> {
    let (_, vchild) = get_vhost_vchild(name);

    execute_command("ip", &["link", "set", "lo", "up"])?;
    execute_command(
        "ip",
        &["addr", "add", &format!("{child_ip}/24"), "dev", &vchild],
    )?;
    execute_command("ip", &["link", "set", &vchild, "up"])?;
    execute_command("ip", &["route", "add", "default", "via", "10.0.0.1"])?;
    Ok(())
}

fn pivot_root_util(merged: &str) -> Result<()> {
    // pivot_root doesn't change a dir, it swaps mounts
    mount(
        Some(merged),
        merged,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .context("bind mount rootfs failed")?;

    let put_old = format!("{merged}/.old_root");
    fs::create_dir_all(&put_old).context("create .old_root failed")?;

    pivot_root(merged, put_old.as_str()).context("pivot_root failed")?;

    chdir("/").context("chdir failed")?;

    umount2("/.old_root", MntFlags::MNT_DETACH).context("unmount old root failed")?;
    fs::remove_dir("/.old_root").ok();
    Ok(())
}

fn setup_overlay(name: &str, lower: &str) -> Result<String> {
    let base = format!("/run/rcr/{name}");
    let upper = format!("{base}/upper");
    let work = format!("{base}/work");
    let merged = format!("{base}/merged");

    fs::create_dir_all(&upper).context("create upper failed")?;
    fs::create_dir_all(&work).context("create work failed")?;
    fs::create_dir_all(&merged).context("create merged failed")?;

    let options = format!("lowerdir={lower},upperdir={upper},workdir={work}");
    mount(
        Some("overlay"),
        merged.as_str(),
        Some("overlay"),
        MsFlags::empty(),
        Some(options.as_str()),
    )
    .context("overlay mount failed")?;

    Ok(merged)
}

fn drop_capabilities() {
    let caps_to_remove = [
        Capability::CAP_SYS_MODULE, // load/unload kernel modules
        Capability::CAP_SYS_RAWIO,  // raw device I/O
        Capability::CAP_SYS_TIME,   // change system clock
        Capability::CAP_SYS_ADMIN,  // huge catch-all power
        Capability::CAP_SYS_PTRACE, // trace/inspect other processes
        Capability::CAP_MKNOD,      // create device nodes
        Capability::CAP_SYS_BOOT,   // reboot the machine
    ];

    for cap in caps_to_remove {
        let _ = caps::drop(None, CapSet::Bounding, cap);
        let _ = caps::drop(None, CapSet::Inheritable, cap);
        let _ = caps::drop(None, CapSet::Effective, cap);
        let _ = caps::drop(None, CapSet::Permitted, cap);
    }
}

fn apply_seccomp() -> Result<()> {
    let mut filter =
        ScmpFilterContext::new(ScmpAction::Allow).context("failed to create seccomp filter")?;

    let blocked = ["reboot", "swapon", "swapoff", "mount", "umount2"];

    for name in blocked {
        let sc = ScmpSyscall::from_name(name).context("invalid syscall name")?;
        filter
            .add_rule(ScmpAction::Errno(libc::EPERM), sc)
            .context("add seccomp rule failed")?;
    }

    filter.load().context("failed to load filter")?;
    Ok(())
}
