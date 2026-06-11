mod cli;
mod state;

use std::ffi::CString;
use std::fs::{self, File};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};

use crate::state::{ContainerState, allocate_ip, list, load, remove, save};
use caps::{CapSet, Capability};
use clap::Parser;
use cli::{Cli, Commands};
use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};
use nix::libc;
use nix::mount::{MsFlags, mount};
use nix::sched::{CloneFlags, unshare};
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, Pid, chdir, chroot, execvp, fork, pipe, read, sethostname, write};

const ROOTFS: &str = "/home/andreastrolle.guest/rootfs";

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            name,
            detach,
            command,
        } => orchestrator(&name, detach, command),
        Commands::List => list_containers(),
        Commands::Stop { name } => stop_container(&name),
        Commands::Logs { name } => logs(&name),
        Commands::SpawnChildContainer {
            name,
            child_ip,
            netns_ready_fd,
            net_configured_fd,
            command,
        } => run_container(&name, &child_ip, netns_ready_fd, net_configured_fd, command),
    }
}

fn list_containers() {
    let containers = list();

    if containers.is_empty() {
        println!("No containers.");
        return;
    }

    println!("{:<15} {:<8} {:<12} {}", "NAME", "PID", "IP", "STATUS");
    for c in containers {
        // Signal 0 probes, does not kill
        let container_is_alive = kill(Pid::from_raw(c.pid), None).is_ok();

        let status = if container_is_alive {
            "running"
        } else {
            "dead"
        };
        println!("{:<15} {:<8} {:<12} {}", c.name, c.pid, c.ip, status);
    }
}

fn stop_container(name: &str) {
    let Some(container) = load(name) else {
        println!("No container named {name}.");
        return;
    };

    let _ = kill(Pid::from_raw(container.pid), Signal::SIGTERM);

    cleanup(name);
    println!("Stopped {name}.")
}

fn logs(name: &str) {
    match fs::read_to_string(format!("/run/rcr/{name}.log")) {
        Ok(content) => print!("{content}"),
        Err(_) => println!("No logs for {name}."),
    }
}

fn orchestrator(name: &str, detach: bool, command: Vec<String>) {
    // child -> orchestrator: netns created
    let (netns_ready_r, netns_ready_w) = pipe().expect("pipe netns_ready failed");
    // orchestrator -> child: network configure, execute now
    let (net_configured_r, net_configured_w) = pipe().expect("pipe net_configured failed");

    let allocated_child_ip = allocate_ip();

    let stdout: Stdio;
    let stderr: Stdio;
    if detach {
        let log = File::create(format!("/run/rcr/{name}.log")).expect("failed to create log");
        let log_err = log.try_clone().expect("failed to clone stdout to stderr");
        stdout = std::process::Stdio::from(log);
        stderr = std::process::Stdio::from(log_err);
    } else {
        stdout = std::process::Stdio::inherit();
        stderr = std::process::Stdio::inherit();
    }

    let mut cmd = Command::new("/proc/self/exe");
    cmd.arg("spawn-child-container")
        .arg(name)
        .arg(&allocated_child_ip)
        .arg(netns_ready_w.as_raw_fd().to_string())
        .arg(net_configured_r.as_raw_fd().to_string());

    for arg in command {
        cmd.arg(arg);
    }

    let mut child = cmd
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .expect("failed to spawn child");

    let child_pid = child.id() as i32;

    setup_cgroups(&name, child_pid);

    let mut buf = [0u8; 1];
    read(&netns_ready_r, &mut buf).expect("wait netns_ready failed");
    setup_host_networking(&name, child_pid);

    let (vhost, _) = get_vhost_vchild(&name);
    save(&ContainerState {
        name: name.to_string(),
        pid: child_pid,
        ip: allocated_child_ip,
        vhost: vhost,
    });

    write(&net_configured_w, &[1u8]).expect("signal net_configured failed");

    if detach {
        println!("Started {name} in detach mode.");
        return;
    }

    child.wait().expect("wait for child container failed");
    cleanup(name);
}

fn run_container(
    name: &str,
    child_ip: &str,
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

            chroot(ROOTFS).expect("chroot failed");
            chdir("/").expect("chdir failed");

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

fn get_vhost_vchild(name: &str) -> (String, String) {
    return (format!("vh-{name}"), format!("vc-{name}"));
}

fn setup_host_networking(name: &str, child_pid: i32) {
    let (vhost, vchild) = get_vhost_vchild(name);

    execute_command(
        "ip",
        &[
            "link", "add", &vhost, "type", "veth", "peer", "name", &vchild,
        ],
    );
    execute_command(
        "ip",
        &["link", "set", &vchild, "netns", &child_pid.to_string()],
    );
    execute_command("ip", &["addr", "add", "10.0.0.1/24", "dev", &vhost]);
    execute_command("ip", &["link", "set", &vhost, "up"]);

    // enable forwarding + NAT so the container reaches the internet
    let _ = std::fs::write("/proc/sys/net/ipv4/ip_forward", "1");

    let nat_rule_exists = Command::new("iptables")
        .args([
            "-t",
            "nat",
            "-C",
            "POSTROUTING",
            "-s",
            "10.0.0.0/24",
            "-j",
            "MASQUERADE",
        ])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !nat_rule_exists {
        execute_command(
            "iptables",
            &[
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-s",
                "10.0.0.0/24",
                "-j",
                "MASQUERADE",
            ],
        );
    }
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

fn execute_command(cmd: &str, args: &[&str]) {
    let status = Command::new(cmd).args(args).status();
    if !status.unwrap().success() {
        panic!("{cmd} failed with {args:?}");
    }
}

fn setup_cgroups(name: &str, child_pid: i32) {
    let _ = fs::write(
        "/sys/fs/cgroup/cgroup.subtree_control",
        "+memory +cpu +pids",
    );
    let _ = fs::create_dir_all("/sys/fs/cgroup/rcr");
    let _ = fs::write(
        "/sys/fs/cgroup/rcr/cgroup.subtree_control",
        "+memory +cpu +pids",
    );

    let cgroup = format!("/sys/fs/cgroup/rcr/{name}");

    fs::create_dir_all(&cgroup).expect("creating cgroup dir failed");

    fs::write(format!("{cgroup}/memory.max"), "104857600").expect("setting memory.max failed");
    fs::write(format!("{cgroup}/pids.max"), "20").expect("setting pids.max failed");
    fs::write(format!("{cgroup}/cpu.max"), "20000 100000").expect("setting cpu.max failed");

    fs::write(format!("{cgroup}/cgroup.procs"), child_pid.to_string())
        .expect("adding child to cgroup failed");
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

fn cleanup(name: &str) {
    let (vhost, _) = get_vhost_vchild(name);

    let _ = Command::new("ip")
        .args(["link", "delete", &vhost])
        .stderr(std::process::Stdio::null())
        .status();

    let _ = fs::remove_dir(format!("/sys/fs/cgroup/rcr/{name}"));

    remove(name);
}
