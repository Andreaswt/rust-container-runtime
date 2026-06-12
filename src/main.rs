mod cli;
mod container;
mod host;
mod image;
mod state;
mod utils;

use crate::container::run_container;
use crate::host::{list_containers, logs, orchestrator, stop_container};
use crate::image::Image;
use clap::Parser;
use cli::{Cli, Commands};

const ROOTFS: &str = "/home/andreastrolle.guest/rootfs";

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            name,
            detach,
            image,
            command,
        } => {
            let lower = match image {
                Some(img) => std::fs::read_to_string(format!("/var/lib/rcr/images/{img}"))
                    .expect("image not found"),
                None => ROOTFS.to_string(),
            };

            orchestrator(&name, &lower, detach, true, command)
        }
        Commands::Build { name } => {
            Image::base("alpine")
                .run("apk add --no-cache python3")
                .run("apk add bird")
                .run("apk add dog")
                .build(&name);
        }
        Commands::List => list_containers(),
        Commands::Stop { name } => stop_container(&name),
        Commands::Logs { name } => logs(&name),
        Commands::SpawnChildContainer {
            name,
            child_ip,
            lower,
            netns_ready_fd,
            net_configured_fd,
            command,
        } => run_container(
            &name,
            &child_ip,
            &lower,
            netns_ready_fd,
            net_configured_fd,
            command,
        ),
    }
}
