use std::fs;

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
) {
    unshare(
        CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWNET,
    )
    .expect("unshare failed");

    let netns_ready_w = unsafe { OwnedFd::from_raw_fd(netns_ready_w) };
    write(&netns_ready_w, &[1u8]).expect("signal netns_ready failed");

    match unsafe { fork() }.expect("fork failed") {
        #[allow(unreachable_code)]
        ForkResult::Child => {
            let net_configured_r = unsafe { OwnedFd::from_raw_fd(net_configured_r) };
            let mut buf = [0u8; 1];
            read(&net_configured_r, &mut buf).expect("wait net_configured failed");

            mount(
                None::<&str>,
                "/",
                None::<&str>,
                MsFlags::MS_REC | MsFlags::MS_PRIVATE,
                None::<&str>,
            )
            .expect("make / private mounts failed");

            setup_container_networking(&name, &child_ip);

            sethostname(&name).expect("sethostname failed");

            // Join cgroup, then chroot removes privileges to unjoin
            std::fs::write(format!("/sys/fs/cgroup/rcr/{name}/cgroup.procs"), "0")
                .expect("self-join cgroup failed");

            let merged = setup_overlay(name, lower);
            setup_dns(&merged);
            pivot_root_util(&merged);

            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::empty(),
                None::<&str>,
            )
            .expect("proc mount failed");

            drop_capabilities();
            apply_seccomp();

            let args: Vec<CString> = command
                .iter()
                .map(|s| CString::new(s.as_str()).unwrap())
                .collect();

            execvp(&args[0], &args).expect("program execution failed");
        }
        ForkResult::Parent { child } => {
            waitpid(child, None).expect("waitpid failed");
        }
    }
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

fn setup_container_networking(name: &str, child_ip: &str) {
    let (_, vchild) = get_vhost_vchild(name);

    execute_command("ip", &["link", "set", "lo", "up"]);
    execute_command(
        "ip",
        &["addr", "add", &format!("{child_ip}/24"), "dev", &vchild],
    );
    execute_command("ip", &["link", "set", &vchild, "up"]);
    execute_command("ip", &["route", "add", "default", "via", "10.0.0.1"]);
}

fn pivot_root_util(merged: &str) {
    // pivot_root doesn't change a dir, it swaps mounts
    mount(
        Some(merged),
        merged,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .expect("bind mount rootfs failed");

    let put_old = format!("{merged}/.old_root");
    fs::create_dir_all(&put_old).expect("create .old_root failed");

    pivot_root(merged, put_old.as_str()).expect("pivot_root failed");

    chdir("/").expect("chdir failed");

    umount2("/.old_root", MntFlags::MNT_DETACH).expect("unmount old root failed");
    fs::remove_dir("/.old_root").ok();
}

fn setup_overlay(name: &str, lower: &str) -> String {
    let base = format!("/run/rcr/{name}");
    let upper = format!("{base}/upper");
    let work = format!("{base}/work");
    let merged = format!("{base}/merged");

    fs::create_dir_all(&upper).expect("create upper failed");
    fs::create_dir_all(&work).expect("create work failed");
    fs::create_dir_all(&merged).expect("create merged failed");

    let options = format!("lowerdir={lower},upperdir={upper},workdir={work}");
    mount(
        Some("overlay"),
        merged.as_str(),
        Some("overlay"),
        MsFlags::empty(),
        Some(options.as_str()),
    )
    .expect("overlay mount failed");

    merged
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

fn apply_seccomp() {
    let mut filter =
        ScmpFilterContext::new(ScmpAction::Allow).expect("failed to create seccomp filter");

    let blocked = ["reboot", "swapon", "swapoff", "mount", "umount2"];

    for name in blocked {
        let sc = ScmpSyscall::from_name(name).expect("invalid syscall name");
        filter
            .add_rule(ScmpAction::Errno(libc::EPERM), sc)
            .expect("add seccomp rule failed");
    }

    filter.load().expect("failed to load filter");
}
