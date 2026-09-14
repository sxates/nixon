#!/bin/bash

# Exit on error
set -e

# Add log level selector with default to INFO
LOG_LEVEL=${1:-info}

case $LOG_LEVEL in
    info|debug|trace)
        export RUST_LOG=$LOG_LEVEL
        ;;
    *)
        echo "Invalid log level: $LOG_LEVEL. Valid options: info, debug, trace"
        exit 1
        ;;
esac

# Clean up previous builds
echo "Cleaning up previous builds..."
#rm -rf target/
#rm -rf src-tauri/target
#rm -rf src-tauri/gen

# Clean up npm, pnp and next
echo "Cleaning up npm, pnp and next..."
rm -rf node_modules
rm -rf .next
rm -rf .pnp.cjs
rm -rf out

echo "Installing dependencies..."
pnpm install

# Build the Next.js application first
echo "Building Next.js application..."
pnpm run build

# Set environment variables for the build
echo "Setting up build environment..."

echo "Building Tauri app..."
# `tauri:dev` (NOT a bare `tauri dev`): package.json maps "tauri" to the raw CLI, which uses
# the base tauri.conf.json and therefore the PRODUCTION identifier ai.vinyl.app — so this
# gate used to read/write the user's real meetings and apply the branch's migrations to the
# production DB (ADR-0004 isolation defeated; a later launch of the released app then fails
# sqlx's VersionMissing check). `tauri:dev` goes through scripts/tauri-auto.js, which merges
# tauri.dev.conf.json for ai.vinyl.app.debug. dev-gpu.sh already does this correctly.
pnpm run tauri:dev
sleep

