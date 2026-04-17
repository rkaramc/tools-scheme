# tools-scheme

A collection of tools and utilities for managing Scheme, Racket, and other Lisp-like languages across different editors. This project builds on existing Racket tooling (`racket-langserver`) to provide a streamlined experience with a focus on REPL-like evaluation.

## Project Goals

- **Evaluation-Centric LSP:** A Rust-based LSP server designed for rapid iteration, providing real-time code evaluation and REPL-like feedback directly in the editor.
- **Universal Editor Support:** Native-feeling extensions and configurations for modern editors, aiming to bridge the gap between heavy IDEs and minimalist text editors.

## Repository Structure

- `lsp/`: The core LSP implementation in Rust (`scheme-toolbox-lsp`).
- `editors/`: Editor-specific logic.
  - `vscode/`: VS Code extension source and metadata.
  - `helix/`: Config fragments and instructions for Helix.
  - `zed/`: Extension support for Zed.
- `specs/`: Design specifications, Architecture Decision Records (ADRs), and project documentation.

## Getting Started

### Prerequisites

- **Racket**: Version 8.0 or later (racket-langserver recommended).
- **Rust**: Latest stable version.
- **Node.js & npm**: Required for building editor extensions.
- **just**: Command runner for standardized build tasks.
- **vsce**: Required for packaging VS Code extensions (`npm install -g @vscode/vsce`).

### Development Workflow

Use `just` to orchestrate builds across the Rust and Node.js components.

```powershell
# Build both the LSP and the VS Code extension in debug mode
just debug
```

#### Debugging the VS Code Extension

To debug the extension during development:

1. Open the `editors/vscode` directory in a new VS Code window.
2. Press `F5` to start the **Run Extension** launch configuration.
3. Use the resulting **[Extension Development Host]** window to test the extension with Scheme/Racket files.

### Installation

To install the LSP globally and side-load the VS Code extension:

```powershell
just install
```

## Contributing

This repository is designed to be agent-friendly. If you are using an AI assistant (like Antigravity or Cursor), please refer to [AGENTS.md](./AGENTS.md) for specialized workflows and documentation standards.

## Status

- **LSP Server:** Active development (Rust 2024). Supporting diagnostics and inlay hints.
- **Editor Support:**
  - **VS Code:** Active development / Beta
  - **Helix:** Planning
  - **Zed:** Planning
