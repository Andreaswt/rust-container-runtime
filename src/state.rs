use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const STATE_DIR: &str = "/run/rcr";

#[derive(Serialize, Deserialize, Debug)]
pub struct ContainerState {
    pub name: String,
    pub pid: i32,
    pub ip: String,
    pub vhost: String, // host-side veth name, for cleanup on stop
}

fn state_path(name: &str) -> PathBuf {
    PathBuf::from(STATE_DIR).join(format!("{name}.json"))
}

pub fn save(state: &ContainerState) -> Result<()> {
    fs::create_dir_all(STATE_DIR).context("create state dir failed")?;
    let json = serde_json::to_string_pretty(state).context("serialize state failed")?;
    fs::write(state_path(&state.name), json).context("write state failed")?;
    Ok(())
}

pub fn load(name: &str) -> Option<ContainerState> {
    let data = fs::read_to_string(state_path(name)).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn remove(name: &str) {
    let _ = fs::remove_file(state_path(name));
}

pub fn list() -> Vec<ContainerState> {
    let mut res: Vec<ContainerState> = Vec::new();

    if let Ok(entries) = fs::read_dir(STATE_DIR) {
        for entry in entries.flatten() {
            // flatten silently drops errors
            if let Ok(file) = fs::read_to_string(entry.path()) {
                if let Ok(state) = serde_json::from_str(&file) {
                    res.push(state);
                }
            }
        }
    }

    res
}

pub fn allocate_ip() -> String {
    let used: Vec<String> = list().iter().map(|x| x.ip.clone()).collect();
    for i in 2..255 {
        let new_ip = format!("10.0.0.{i}");
        if !used.contains(&new_ip) {
            return new_ip;
        }
    }

    panic!("all ip addresses taken in 10.0.0.0/24");
}
