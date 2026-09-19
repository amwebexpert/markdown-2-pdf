# md2pdf — common dev tasks (https://github.com/casey/just)

default:
    @just --list

# --- Build ---

# Debug-build the native CLI (`target/debug/md2pdf`).
build-cli:
    cargo build -p md2pdf

# Release-build the native CLI.
build-cli-release:
    cargo build -p md2pdf --release

# Build the browser WASM package into `wasm/pkg/` (gitignored).
build-wasm:
    wasm-pack build wasm --target web

# WASM build with npm scope for publishing `@amwebexpert/md2pdf-wasm`.
build-wasm-npm:
    wasm-pack build wasm --target web --scope amwebexpert

# --- CLI convert ---

# Convert a Markdown file; output defaults to same path with `.pdf`.
convert input *FLAGS='':
    cargo run -p md2pdf -- convert {{input}} {{FLAGS}}

# Regenerate PDFs for all `cli/examples/*.md`.
examples:
    #!/usr/bin/env bash
    set -euo pipefail
    for f in cli/examples/*.md; do
      cargo run -p md2pdf -- convert "$f"
    done

# --- Demo ---

# Build WASM, install demo deps, run Vite dev server.
demo: build-wasm
    cd demo && bun install && bun start

# Production build of the demo static site (`demo/dist/`).
demo-build: build-wasm
    cd demo && bun install && bun run build

# --- Quality ---

check:
    cargo check --workspace

clippy:
    cargo clippy --workspace -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# --- Publish ---

# Publish `@amwebexpert/md2pdf-wasm` to npm (login required).
publish-npm: build-wasm-npm
    cd wasm/pkg && npm publish --access public

# --- Clean ---

clean:
    cargo clean
    rm -rf wasm/pkg demo/node_modules demo/dist demo/*.tsbuildinfo
