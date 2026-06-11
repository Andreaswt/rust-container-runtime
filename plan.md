Stage 1: run a command in a child process. No isolation at all. Just fork + exec. You type a command, it runs. Boring, but it's your skeleton.

Stage 2: give it a root filesystem. Pull/extract a rootfs (you can even do this by hand with curl + tar at first), chroot into it. Now ls / shows Alpine's files, not your host's. First "whoa" moment.

Stage 3: isolate the process tree. Add the PID namespace. Now you hit the fork requirement and the /proc mount naturally, because ps won't work until you mount it. You learn why those exist by needing them.

Stage 4: isolate hostname, mounts, network. Add the other namespaces one at a time. After the network namespace, the container has no internet, that's your trigger to build veth + bridge.

Stage 5: resource limits. cgroups. Run something that eats all the RAM, watch it take down your VM, then add a memory limit and watch it get killed instead.

Stage 6: security hardening. Now seccomp and capabilities make sense, because you go looking for what a container can still do to the host and lock those down.

Stage 7: the daemonless management layer. State files, list/stop/logs.
