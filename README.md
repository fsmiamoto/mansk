<p align="center">
  <img src="assets/mansk-mascot.png" alt="mansk — manage skills" width="320">
</p>

Reproducible agent-skills manager. Skills are declared in a TOML manifest,
pinned to exact Git commits in `skills.lock`, cached locally, and installed as
symlinks. mansk currently supports Unix systems; CI validates Linux.

## Install

```sh
cargo install --path .
```

## Development

Contributions must pass the repository's formatting, static-analysis, test,
MSRV, and dependency-policy gates. See [CONTRIBUTING.md](CONTRIBUTING.md) for
setup and run the same checks as CI with:

```sh
just check
```

## Usage

The manifest lives at `~/.config/mansk/skills.toml` (or under
`$XDG_CONFIG_HOME` if set); pass `--manifest PATH` before the command to use
another file. `skills.lock` is written next to the manifest.

```sh
mansk update   # resolve selectors, rewrite the lock, install (asks first)
mansk sync     # install exactly what the lock records
```

`update --yes` skips the confirmation; both commands take `--dry-run`.

## Manifest

```toml
schema = 1
default-targets = ["claude", "pi"]

[targets]
claude = ".claude/skills"
pi = ".pi/agent/skills"
custom = "/absolute/path"

# A repository whose direct children containing SKILL.md are all installed.
[[collections]]
source = "https://github.com/example/all-skills.git"
selector = "main"
root = "skills"        # optional, defaults to the repo root

# A single skill from a Git repository.
[[skills]]
source = "https://github.com/example/skills.git"
path = "skills/review"
selector = "v2"        # branch, tag, or commit; pinned by update
targets = ["claude"]   # optional, replaces default-targets

# A local directory, relative to this manifest.
[[skills]]
path = "../local-skill"
```

The schema is strict: unknown fields are errors. `[targets]` maps arbitrary
symbolic names to installation directories. Relative directories resolve
against `$HOME`; absolute directories are used as written. Paths do not expand
`~`, environment variables, or shell syntax. `default-targets` and each
skill's optional `targets` list refer to names in this map, and an unknown name
is an error. A skill's `targets` replaces the defaults; collections use the
defaults.

Every skill directory must contain a `SKILL.md`; its directory name is the
installed name, and duplicate names are rejected. Local skills take only
`path` and optional `targets`; Git skills require `source`, `path`, and
`selector`.

## How it works

`update` resolves branches and tags to exact commits and records them in
`skills.lock`; `sync` never advances them, so commit the lock if you want
reproducible installs. Content is cached under `~/.cache/mansk` — safe to
delete, the next sync rebuilds it — and installed skills are symlinks into
that cache.

mansk only manages symlinks that point into its own cache. Those are created,
relinked, and pruned as the manifest changes; anything else in a target
directory is left alone, and a name collision with an unmanaged entry is an
error, never an overwrite. Every directory declared under `[targets]` is
scanned, even if no current skill refers to it, so stale owned links are pruned
while that declaration remains.
