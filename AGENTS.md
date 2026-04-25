# Agents in Tools Scheme

Welcome to the **Tools Scheme** repository. This document provides essential context and instructions for AI coding agents (like Antigravity) working on this codebase.

## Repository Structure

- **`lsp/`**: Rust implementation of the Language Server Protocol for Racket.
- **`editors/vscode/`**: VS Code extension source code.

## Tooling

- **`.agents/workflows/`**: Machine-readable and human-readable guides for specific tasks.
- **`.beads/`**: Issue tracking using the [Beads](https://github.com/steveyegge/beads) system. (Agents MUST use the non-interactive `bd` command for issues, e.g., `bd list` or `bd show`. Do not use the interactive `bv` command.)

> [!NOTE]
> Tooling preferences (e.g., JJ vs Git) are managed via personal overrides in `.agents/local.md`. Agents should check for that file for user-specific workflow instructions.

## High-Level Workflows

Agents should favor following standardized workflows located in `.agents/workflows/`. Specifically:

- **Bug Fixing**: Use the `fix-bead.md` workflow (triggered via `/fix-bead <issue-id>`). This workflow emphasizes investigation, TDD, and JJ-based context management.

## Karpathy Guidelines (Repository-Wide)

- **Simplicity First**: Implement ONLY what is requested.
- **Surgical Changes**: Minimize diff noise. Match existing style and don't refactor adjacent code.
- **Goal-Driven**: Use failing tests to define success before implementing fixes.
- **Direct Edits**: Edit the source of truth directly; avoid fragmented documentation.
