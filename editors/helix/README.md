# Helix Integration for Tools Scheme

This directory contains the reference configuration and documentation for using `scheme-toolbox-lsp` within the [Helix editor](https://helix-editor.com/).

## Overview

Helix is a modal, terminal-based editor with built-in Language Server Protocol (LSP) support. By configuring Helix to use `scheme-toolbox-lsp`, you get immediate execution feedback, inlay hints, and Racket/Scheme notebook capabilities directly in your terminal.

## Quick Start

### 1. Prerequisites

Ensure you have the following installed:
- **Racket**: The core runtime (`racket` in your PATH).
- **Helix**: The editor (`hx` command).
- **scheme-toolbox-lsp**: Build and install using the project root `justfile`:
  ```bash
  just install
  ```
- **racket-langserver** (Optional): For deep static analysis:
  ```bash
  raco pkg install racket-langserver
  ```

### 2. Automatic Configuration

The easiest way to configure Helix is to use the provided automation in the project's `justfile`:

```bash
just configure-helix
```

This command will automatically detect your Helix configuration directory and append the necessary settings to your `languages.toml`.

### 3. Verify Setup

Check if Helix correctly detects the server:
```bash
hx --health racket
```

## Features

### Inlay Hints (Live Evaluation)
As you type, `scheme-toolbox-lsp` evaluates your code in a background Racket sandbox and displays the result as an "inlay hint" next to the expression.

### Notebook Mode (`.rktnb` / `.scmnb`)
Files with these extensions are treated as "Notebooks".
- **Separators**: Use **two or more newlines** to separate blocks.
- **Feedback**: Each block gets its own evaluation result hint.
- **Compatibility**: These are standard Racket files and can be executed by the `racket` CLI normally.

### Multi-LSP Support
The default configuration enables **both** `scheme-toolbox-lsp` (for live feedback) and `racket-langserver` (for static analysis). Helix merges the diagnostics and hints from both, providing a powerful development environment.

## Customization

You can manually edit your `languages.toml` (located in `~/.config/helix/` or `%AppData%\helix\`) using the reference [languages.toml](./languages.toml) found in this directory.

### Disabling racket-langserver
If you find the dual-LSP setup too heavy, you can restrict the servers in your `languages.toml`:

```toml
[[language]]
name = "racket"
language-servers = [ "scheme-toolbox-lsp" ]
```

### Suppressing Redundant Diagnostics
If you keep both servers enabled, you can disable diagnostic publishing in `scheme-toolbox-lsp` to avoid duplicate error markers:

```toml
[language-server.scheme-toolbox-lsp.config]
disableDiagnostics = true
```

## Troubleshooting
If inlay hints are not appearing, ensure they are enabled in your Helix `config.toml`:

```toml
[editor]
lsp.display-inlay-hints = true
```

For more detailed information, see the full [Helix Integration Guide](../../docs/helix-integration.md).
