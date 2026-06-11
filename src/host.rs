use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::process::{Command, Stdio};

use crate::state::{ContainerState, allocate_ip, save};
use crate::state::{list, load};
use crate::utils::{cleanup, execute_command, get_vhost_vchild};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use nix::unistd::{pipe, read, write};

pub fn orchestrator(name: &str, detach: bool, command: Vec<String>) {
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

pub fn list_containers() {
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

pub fn stop_container(name: &str) {
    let Some(container) = load(name) else {
        println!("No container named {name}.");
        return;
    };

    let _ = kill(Pid::from_raw(container.pid), Signal::SIGTERM);

    cleanup(name);
    println!("Stopped {name}.")
}

pub fn logs(name: &str) {
    match fs::read_to_string(format!("/run/rcr/{name}.log")) {
        Ok(content) => print!("{content}"),
        Err(_) => println!("No logs for {name}."),
    }
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
