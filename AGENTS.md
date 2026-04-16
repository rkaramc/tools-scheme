# Agents in Tools Scheme

Welcome to the **Tools Scheme** repository. This document provides essential context and instructions for AI coding agents (like Antigravity) working on this codebase.

## Repository Structure

- **`lsp/`**: Rust implementation of the Language Server Protocol for Racket.
- **`editors/vscode/`**: VS Code extension source code.
- **`.agents/workflows/`**: Machine-readable and human-readable guides for specific tasks.
- **`.beads/`**: Issue tracking using the [Beads](https://github.com/steveyegge/beads) system.

## Tooling Standards

- **Version Control**: Use `jj` (Jujutsu) by default. Avoid `git` unless explicitly necessary.
- **Package Management**: Use `uv` for all Python-related tasks.
- **Issue Tracking**: Use the `bd` CLI to interact with the Beads database (e.g., `bd list`, `bd show <id>`).

## High-Level Workflows

Agents should favor following standardized workflows located in `.agents/workflows/`. Specifically:

- **Bug Fixing**: Use the `fix-bead.md` workflow (triggered via `/fix-bead <issue-id>`). This workflow emphasizes investigation, TDD, and JJ-based context management.

## Karpathy Guidelines (Repository-Wide)

- **Simplicity First**: Implement ONLY what is requested.
- **Surgical Changes**: Minimize diff noise. Match existing style and don't refactor adjacent code.
- **Goal-Driven**: Use failing tests to define success before implementing fixes.
- **Direct Edits**: Edit the source of truth directly; avoid fragmented documentation.
