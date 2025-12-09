#!/bin/bash
# Install Collar Daemon on your computer
# Run this on your desktop/laptop, NOT the server

set -e

echo "==> Building collar-daemon..."
cargo build --release -p collar-daemon

echo "==> Installing binary..."
mkdir -p ~/.local/bin
cp target/release/collar-daemon ~/.local/bin/

echo "==> Creating config directory..."
mkdir -p ~/.config/collar

if [ ! -f ~/.config/collar/config.toml ]; then
    echo "==> Copying example config..."
    cp collar.example.toml ~/.config/collar/config.toml
    echo ""
    echo "!!! IMPORTANT: Edit ~/.config/collar/config.toml"
    echo "    Set your device_key from server.toml"
    echo ""
fi

echo "==> Installing systemd user service..."
mkdir -p ~/.config/systemd/user
cp deploy/collar-daemon.service ~/.config/systemd/user/

echo "==> Enabling service..."
systemctl --user daemon-reload
systemctl --user enable collar-daemon

echo ""
echo "==> Done! To start:"
echo "    systemctl --user start collar-daemon"
echo ""
echo "    View logs:"
echo "    journalctl --user -u collar-daemon -f"
