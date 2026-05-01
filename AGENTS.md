# Agents in Tools Scheme

Welcome to the **Tools Scheme** repository. This document provides essential context and instructions for AI coding agents working on this codebase.

## Tech Stack

- **Backend (LSP)**: Rust (2024 edition), `lsp-server`, `crossbeam-channel`. Integrates with Racket for evaluation.
- **Frontend (VS Code)**: TypeScript, Node.js, VS Code Extension API. Testing via Jest and `@vscode/test-electron`.
- **Infrastructure**: [Just](https://github.com/casey/just) task runner, [Jujutsu](https://docs.jj-vcs.dev/) (jj) & Git VCS, [Beads](https://github.com/steveyegge/beads) issue tracking.

## Repository Structure

- **`lsp/`**: Rust implementation of the Language Server Protocol for Racket.
- **`editors/vscode/`**: VS Code extension source code.

## Code Conventions

- **Rust**: Follow idiomatic Rust coding and formatting (`cargo fmt`). Keep things simple and rely on explicit error handling (`anyhow`).
- **TypeScript**: Use strict typing. Avoid `any`. Run `npm run lint` to verify. Follow existing styles for naming and spacing.

## Boundaries

- Never commit broken builds; verify locally with `just debug` and `just test` / `cargo test` / `npm test`.
- Do not refactor adjacent code or modify architectural designs unless explicitly instructed.

## Tooling

- **Just**: Task runner using the [Just]() tool. See `justfile` for available commands (build, package, install, test).
- **Beads**: Issue tracking using the [Beads](https://github.com/steveyegge/beads) system. Agents should use the `beads-tracking` skill.
- **Jujutsu**: Version control using the [Jujutsu](https://docs.jj-vcs.dev/) system (also known as `jj` or `jj-vcs`). Agents should use the `jj-vcs` skill.

> [!NOTE]
> Tooling preferences (e.g., JJ vs Git, Beads vs Markdown Todos) are managed via personal overrides in `.agents/local.md`. Agents should check for that file for user-specific workflow instructions.

## High-Level Workflows

Agents should favor following standardized workflows located in `.agents/workflows/`. Specifically:

- **Fix Bugs/Perform Tasks**: Use the `fix-bead.md` workflow (triggered via `/fix-bead <issue-id>`).
- **Commit Changes**: Use the `commit-changes.md` workflow (triggered via `/commit-changes`).

If the user has not triggered the workflow manually, but the agent determines that it is appropriate, ask user if the workflow should be applied.
