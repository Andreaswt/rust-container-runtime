# rcr

A Linux container runtime built from scratch in Rust. Namespaces, cgroups, seccomp, capabilities, OverlayFS, and a daemonless management layer, no Docker involved.

```bash
# start the VM (Apple Silicon, via Lima)
limactl start --name=rcr lima.yaml
limactl shell rcr

# build and run
rcr run --name my-shell --image alpine /bin/sh

# just run without rebuilding
rcrx run --name my-shell --image alpine /bin/sh
```

## Commands

```
rcr run --name <name> [--image <image>] [--detach] [cmd]   run a container
rcr build <name>                                            build an image
rcr list                                                     list containers
rcr stop <name>                                              stop a container
rcr logs <name>                                               read logs from a detached container
```

## Build an image

```rust
Image::base("alpine")
    .run("apk add --no-cache python3")
    .copy("./app", "/app")
    .build("myimage");
```

```bash
rcr run --image myimage python3 /app/script.py
```
