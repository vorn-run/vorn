#!/usr/bin/env bash
# Prepares a fresh Ubuntu 24.04 bench host: toolchains, then the kernel,
# rlimit and systemd limits that thousands of terminals run into first.
# Limits take effect after the reboot bench-gcp.sh does next.
set -euo pipefail

ZIG_VERSION=0.15.2
RUST_VERSION=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$HOME/vorn/packages/core/rust-toolchain.toml")

sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
  build-essential pkg-config git curl xz-utils jq >/dev/null

if ! command -v rustup >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_VERSION"
fi

if [ ! -x "$HOME/zig/zig" ]; then
  arch=$(uname -m)
  curl -sSfL "https://ziglang.org/download/$ZIG_VERSION/zig-$arch-linux-$ZIG_VERSION.tar.xz" -o /tmp/zig.tar.xz
  mkdir -p "$HOME/zig"
  tar -xJf /tmp/zig.tar.xz -C "$HOME/zig" --strip-components=1
fi
"$HOME/zig/zig" version | grep -qx "$ZIG_VERSION"

sudo tee /etc/sysctl.d/90-vorn-bench.conf >/dev/null <<'EOF'
kernel.pty.max = 65536
kernel.pid_max = 4194304
kernel.threads-max = 4194304
vm.max_map_count = 1048576
fs.nr_open = 1048576
fs.file-max = 4194304
fs.inotify.max_user_instances = 65536
EOF
sudo sysctl -q --system

sudo tee /etc/security/limits.d/90-vorn-bench.conf >/dev/null <<'EOF'
* soft nofile 1048576
* hard nofile 1048576
* soft nproc unlimited
* hard nproc unlimited
EOF

# systemd caps tasks per user slice, per user manager and per unit; vornd
# starts the holder in a user scope, so all three must go.
sudo mkdir -p /etc/systemd/system.conf.d /etc/systemd/user.conf.d \
  /etc/systemd/system/user-.slice.d /etc/systemd/system/user@.service.d
printf '[Manager]\nDefaultTasksMax=infinity\nDefaultLimitNOFILE=1048576\n' |
  sudo tee /etc/systemd/system.conf.d/90-vorn-bench.conf /etc/systemd/user.conf.d/90-vorn-bench.conf >/dev/null
printf '[Slice]\nTasksMax=infinity\n' | sudo tee /etc/systemd/system/user-.slice.d/90-vorn-bench.conf >/dev/null
printf '[Service]\nTasksMax=infinity\nLimitNOFILE=1048576\n' |
  sudo tee /etc/systemd/system/user@.service.d/90-vorn-bench.conf >/dev/null
# The user manager outlives ssh sessions, so the holder's scope has one.
sudo loginctl enable-linger "$USER"
echo "setup done"
