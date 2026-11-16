# Yunari (native)

Native Windows rewrite of Yunari in Rust + gpui-kit.

Design: `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`
Phase 0 plan: `docs/superpowers/plans/2026-11-16-yunari-phase-0-foundation.md`

## Requirements

- Windows 10 or later, MSVC Rust toolchain (1.92+)
- Visual Studio 2022 Build Tools with **Desktop development with C++**
- CMake on `PATH` (gpui-kit build dependency)

`scripts/dev-setup.ps1` checks all of the above and primes the cargo cache.
Orca runs it for every new worktree through `orca.yaml`.

Where MSVC is not installed and installing it is not an option, the same script
writes `.cargo/config.toml` pointing at a portable toolchain instead — clang,
`lld-link`, and the Windows SDK's own headers and import libraries, all under
`%LOCALAPPDATA%` with no administrator rights and nothing registered on the
machine. That toolchain is provisioned once with:

```powershell
pwsh -File scripts/dev-setup.ps1 -Provision
```

`.cargo/config.toml` is machine state, so it is not committed; every later
`cargo` command picks it up on its own.

## Commands

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p yunari-app
```

Running, uninstalling and where state lives: `docs/install.md`. A release build
additionally needs the Windows SDK's `fxc.exe`, because GPUI compiles its
shaders only when debug assertions are off.

## Layout

| Crate | Role |
|---|---|
| `crates/model` | Versioned theme document, parsing and validation. No UI, OS or network dependencies. |
| `crates/render` | Renderer-independent decisions: formatting, transforms, colours, display lists. No backend. |
| `crates/raster` | Display list to pixels (tiny-skia): panels, and the reference every golden compares against. |
| `crates/theme` | Editor design tokens (spacing, radius, type scale). |
| `crates/golden` | Pixel golden-file harness for render tests. |
| `crates/app` | gpui-kit application shell. |
