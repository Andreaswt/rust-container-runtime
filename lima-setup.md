## Start a dev session

```bash
limactl start rcr     # if not already running
limactl shell rcr     # enter the VM
rcr run --name my-shell --image alpine /bin/sh
```

Edit code in VS Code on the Mac; `rcr` rebuilds and runs. The image stays
cached, so no re-pull needed unless you cleared it or changed image URLs.

## Setup

Create lima.yaml file:

```yaml
images:
  - location: "https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-arm64.img"
    arch: "aarch64"

mounts:
  - location: "~/Documents/Repositories/rust-container-runtime"
    writable: true
```

```bash
# Mac
brew install lima
limactl start --name=rcr lima.yaml
limactl shell rcr

# Inside the VM
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libseccomp-dev iproute2 iptables python3
```

Add to `~/.bashrc` (VM), then `source ~/.bashrc`:

```bash
cat >> ~/.bashrc << 'EOF'
export CARGO_TARGET_DIR=$HOME/rcr-target
alias rcr='cd /Users/andreastrolle/Documents/Repositories/rust-container-runtime && cargo build --release && cd ~ && sudo $HOME/rcr-target/release/rust-container-runtime'
alias rcrx='sudo $HOME/rcr-target/release/rust-container-runtime'
EOF
```

```bash
source ~/.bashrc
```

## Run

```bash
rcr: # build and run
rcrx: # just run
```

```bash
cd ~
rcrx pull alpine                                  # one-time
rcr run --name my-shell --image alpine /bin/sh    # edit on Mac, run in VM
```

Inspect from a second `limactl shell rcr`: `rcrx list`, `rcrx logs <name> --follow`, `rcrx stop <name>`.
