set shell := ["bash", "-euo", "pipefail", "-c"]

# Show the available development commands.
default:
    @just --list

# Install the pinned auxiliary tools and the declared minimum Rust version.
setup:
    rustup toolchain install 1.85.0 --profile minimal
    cargo install --locked --version 0.20.2 cargo-deny
    cargo install --locked --version 0.9.2 cargo-machete

# Apply repository formatting.
fmt:
    cargo fmt --all
    just --fmt

# Verify Rust and Justfile formatting without changing files.
fmt-check:
    cargo fmt --all -- --check
    just --fmt --check

# Compile every target and feature with the committed dependency graph.
compile:
    cargo check --locked --all-targets --all-features

# Run strict Clippy across all targets and production panic-safety checks.
lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo clippy --locked --bin mansk --all-features -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic

# Run the complete test suite.
test:
    cargo test --locked --all-targets --all-features

# Treat rustdoc diagnostics as build failures.
docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features --document-private-items

# Verify compatibility with the minimum Rust version declared in Cargo.toml.
msrv:
    rustup run 1.85.0 cargo check --locked --all-targets --all-features

# Enforce advisory, license, source, and banned-dependency policy.
deny:
    cargo deny check

# Fail when a direct dependency appears to be unused.
unused-deps:
    cargo machete

# Run every required local and CI quality gate.
check: fmt-check compile lint test docs msrv deny unused-deps
