#!/bin/bash
# Build Collar (run once, then use systemd services)

set -e
cd "$(dirname "$0")/.."

echo "==> Building server..."
cargo build --release -p collar-server

echo "==> Building frontend..."
cd collar-web
npm install
npm run build
cd ..

echo "==> Done! Start services with:"
echo "    sudo systemctl start collar-server collar-web"
