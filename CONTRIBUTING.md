# Contributing

## Prerequisites

mansk currently targets Unix systems, while CI validates Linux specifically.
Install the tools below before working on it:

- [Rustup](https://rustup.rs/)
- [Just](https://github.com/casey/just) 1.57.0

The repository's `rust-toolchain.toml` makes rustup select Rust 1.97.1 with
rustfmt and Clippy. Install the remaining pinned tools and the minimum supported
Rust version once:

```sh
just setup
```

## Required checks

Run the complete build gate before opening a pull request:

```sh
just check
```

The aggregate command uses the committed lockfile and runs formatting checks,
compilation, strict Clippy lints, tests, rustdoc, Rust 1.85 compatibility,
dependency policy, and unused-dependency detection. CI calls the same recipes,
so local and hosted feedback should agree.

Useful focused commands are available with `just --list`. In particular:

```sh
just fmt       # apply Rust and Justfile formatting
just lint      # compiler/default lints plus production panic-safety lints
just test      # all test targets and features
just deny      # advisories, licenses, sources, and banned dependencies
```

Warnings are build failures. Production code must not use unsafe code,
`unwrap`, `expect`, explicit panics, unfinished placeholders, or `dbg!`.
Test setup may use `unwrap` and `expect`, but all other warning and unfinished
code policies still apply.

## Dependency changes

`deny.toml` permits only reviewed permissive licenses and crates.io sources.
A new license family or source needs an explicit policy review; do not add an
exception merely to make CI green. Known vulnerabilities and wildcard
dependency requirements fail the build. Duplicate transitive versions are
reported for review but do not fail because they are not always under this
project's control.

`cargo machete` is heuristic. If generated code or another non-obvious use
causes a false positive, document the reason in Cargo metadata rather than
removing the check.

## CI and merging

Repository administrators should configure the following CI jobs as required
branch-protection checks:

- Formatting
- Static analysis
- Tests
- Rust 1.85 compatibility
- Dependency policy

A workflow being red is only an effective merge gate when these statuses are
required by branch protection.
