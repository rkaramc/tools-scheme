# tools-scheme

A collection of tools and utilities for managing Scheme, Racket, and other Lisp-like languages across different editors. This project aims to provide a robust Language Server Protocol (LSP) implementation and the necessary editor plugins/configurations for a seamless development experience.

## Project Goals

- **Scheme/Racket LSP:** A high-performance LSP server written in Rust to provide intelligent code completion, diagnostics, and navigation for Scheme and Racket variants.
- **Editor Support:** Native-feeling configurations and extensions for modern editors like Helix, VSCode, and Zed.

## Repository Structure

- `lsp/`: The core LSP implementation in Rust (`scheme-toolbox-lsp`).
- `editors/`: Editor-specific configurations and extensions (VSCode, Helix, Zed).
- `specs/`: Design specifications, Architecture Decision Records (ADRs), and project documentation.

## Getting Started

### Prerequisites

- Rust (latest stable)
- `cargo`

### Building the LSP

To build the LSP server:

```powershell
cargo build -p scheme-toolbox-lsp
```

### Development

Run the LSP in development mode:

```powershell
cargo run -p scheme-toolbox-lsp
```

## Status

- **LSP Server:** In initial development (Rust edition 2024).
- **Editor Support:**
  - **Helix:** Planned / In development
  - **VSCode:** Planned / In development
  - **Zed:** Planned / In development
