<p align="center">
  <img src="public/brand/logo-mark.svg" alt="Conclave" width="120" />
</p>

<h1 align="center">Conclave</h1>

<p align="center">
  A native macOS app for orchestrating multi-agent software work.<br />
  Define agents once, launch them in a workspace with the right identity and skills,
  and keep tasks, blackboard, memory and messages visible so a lead can decide.
</p>

## Requirements

| Tool | Version | Notes |
| --- | --- | --- |
| macOS | Apple Silicon | The only supported platform today |
| Node.js | 22+ | |
| pnpm | 9+ | The Tauri config runs `pnpm dev` / `pnpm build`; `bun` and `npm` are not used |
| Rust | 1.96.0 | Pinned in `rust-toolchain.toml` (`rustup` installs it); minimum 1.88 |
| Tauri CLI | 2.x | `cargo install tauri-cli --version "^2"` |
| Agent CLI | any of Claude Code, Codex, Antigravity | At least one, logged in, on your login-shell `PATH` |

## Run

```sh
pnpm install
pnpm tauri dev
```

`pnpm tauri` first stages the pinned `rtk` binary (`scripts/fetch-rtk.sh`, via `cargo install`), then starts the app.

## Build

```sh
pnpm tauri build
```

## Docs

- Product: `PRODUCT.md`
- Agent instructions and UI pixel gate: `CLAUDE.md`
- Design specs and plans: `docs/superpowers/`
- Brand assets: `docs/brand/README.md`
