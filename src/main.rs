mod cli;
mod container;
mod host;
mod state;
mod utils;

use crate::container::run_container;
use crate::host::{list_containers, logs, orchestrator, stop_container};
use clap::Parser;
use cli::{Cli, Commands};

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
