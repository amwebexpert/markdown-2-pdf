# md2pdf — common dev tasks (https://github.com/casey/just)

default:
    @just --list

# --- Build ---

# Debug-build the native CLIs (`target/debug/md2pdf`, `target/debug/pdf2md`).
build-cli:
    cargo build -p md2pdf

# Release-build the native CLIs.
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

# --- CLI pdf2md ---

# Convert a PDF file to Markdown; output defaults to same path with `.md`.
pdf2md input *FLAGS='':
    cargo run -p md2pdf --bin pdf2md -- {{input}} {{FLAGS}}

# Rewrite pdf2md snapshot files (`pdf2md-core/tests/snapshots/`) to current output; review the diff.
snapshots:
    INSTA_UPDATE=always cargo test -p pdf2md-core

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

test:
    cargo test --workspace

# Lint all crates and tests; warnings are errors (config: `[workspace.lints]` in Cargo.toml).
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Lint the WASM crate for its real target.
clippy-wasm:
    cargo clippy -p md2pdf-wasm --target wasm32-unknown-unknown -- -D warnings

# Apply Clippy's automatic fixes.
clippy-fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty

# Licenses, security advisories, banned/duplicate crates (`cargo install --locked cargo-deny`).
deny:
    cargo deny check

# Unused dependencies (`cargo install --locked cargo-machete`).
machete:
    cargo machete

# Full quality gate (same checks as CI).
lint: fmt-check clippy clippy-wasm test deny machete

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
