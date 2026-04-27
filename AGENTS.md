# Agents in Tools Scheme

Welcome to the **Tools Scheme** repository. This document provides essential context and instructions for AI coding agents (like Antigravity) working on this codebase.

## Tech Stack
- **LSP (`lsp/`)**: Rust (2024 edition), `lsp-server`, `crossbeam-channel`. Built using standard cargo build system.
- **VS Code Extension (`editors/vscode/`)**: TypeScript, Node.js, VS Code Extension API.

## Repository Structure
- **`lsp/`**: Rust implementation of the Language Server Protocol for Racket.
- **`editors/vscode/`**: VS Code extension source code.

## Commands
We use `just` as a command runner.
- **Build (Debug)**: `just build 1` or `just debug` (builds both LSP and VS Code extension)
- **Build (Release)**: `just build 2` or `just release`
- **Install locally**: `just install`
- **Test LSP**: `cd lsp; cargo test`
- **Test VS Code Extension**: `cd editors/vscode; npm test` (or `npm run test-integration`)
- **Lint VS Code Extension**: `cd editors/vscode; npm run lint`
- **Compile VS Code Extension**: `cd editors/vscode; npm run compile`

## Code Conventions
- **Rust**: Follow idiomatic Rust formatting (`cargo fmt`). Keep things simple and rely on explicit error handling (`anyhow`).
- **TypeScript**: Use strict typing. Avoid `any`. Run `npm run lint` to verify. Follow existing styles for naming and spacing.

## Boundaries
- Never commit broken builds; verify locally with `just debug` and `cargo test` / `npm test`.
- Do not refactor adjacent code or modify architectural designs unless explicitly instructed.
- Only use Jujutsu (`jj`) or `git` according to the personal overrides.

## Tooling
- **Beads**: Issue tracking using the [Beads](https://github.com/steveyegge/beads) system. Agents should check the `beads-tracking` skill for instructions.
- **Jujutsu**: (also known as `jj` or `jj-vcs`) Version control using the [Jujutsu](https://docs.jj-vcs.dev/) system. Agents should check the `jj-vcs` skill for instructions.

> [!NOTE]
> Tooling preferences (e.g., JJ vs Git) are managed via personal overrides in `.agents/local.md`. Agents should check for that file for user-specific workflow instructions.

## High-Level Workflows
Agents should favor following standardized workflows located in `.agents/workflows/`. Specifically:
- **Bug Fixing**: Use the `fix-bead.md` workflow (triggered via `/fix-bead <issue-id>`). This workflow emphasizes investigation, TDD, and JJ-based context management.

## Karpathy Guidelines (Repository-Wide)
More information in the `karpathy-guidelines` skill.
- **Simplicity First**: Implement ONLY what is requested.
- **Surgical Changes**: Minimize diff noise. Match existing style and don't refactor adjacent code.
- **Goal-Driven**: Use failing tests to define success before implementing fixes.
- **Direct Edits**: Edit the source of truth directly; avoid fragmented documentation.
