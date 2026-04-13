# Spec: Project Initialization and Documentation

## Objective
Initialize the `tools-scheme` workspace with essential project-level documentation (`README.md`). The goal is to provide a clear entry point for developers, explaining the repository structure and the purpose of the tools being developed: an LSP server for Scheme/Racket languages and variants, along with associated editor plugins.

## Tech Stack
- **Languages:** Rust (edition 2024)
- **Target Languages:** Scheme, Racket, and variants.
- **Version Control:** Jujutsu (`jj`)
- **Project Structure:** Cargo Workspace
- **Components:**
  - LSP Server: `scheme-toolbox-lsp` (Rust)
  - Editor Support: Helix, VSCode, Zed

## Commands
- **Build LSP:** `cargo build -p scheme-toolbox-lsp`
- **Run LSP:** `cargo run -p scheme-toolbox-lsp`
- **Lint Workspace:** `cargo clippy --all-targets --all-features`
- **Format Workspace:** `cargo fmt --all`

## Project Structure
```
tools-scheme/
├── Cargo.toml          → Workspace configuration
├── lsp/                → LSP server implementation (Rust)
│   ├── Cargo.toml
│   └── src/main.rs
├── editors/            → Editor-specific configurations/extensions
│   ├── helix/
│   ├── vscode/
│   └── zed/
├── specs/              → Design specifications and ADRs
└── target/             → Build artifacts (ignored)
```

## Success Criteria
- [x] `README.md` file created in the root directory.
- [x] Project objective clearly stated.
- [x] Repository structure documented.
- [x] Core build/run commands included.
- [x] Editor support status mentioned.
