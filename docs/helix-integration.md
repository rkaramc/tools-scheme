# Helix Editor Integration

This document describes how to configure the [Helix editor](https://helix-editor.com/) to use the `scheme-toolbox-lsp` for Racket and Scheme development.

## Prerequisites

- **Racket**: Must be installed and available in your `PATH`.
- **scheme-toolbox-lsp**: Must be installed and available in your `PATH`.
  - Install using `just install`.
- **racket-langserver** (Optional but recommended):
  - Install using `raco pkg install racket-langserver`.
- **Helix**: Must be installed (`hx`).

## Configuration

Helix uses a `languages.toml` file for language-specific configuration. Depending on your OS, this file is located at:

- **Linux/macOS**: `~/.config/helix/languages.toml`
- **Windows**: `%AppData%\helix\languages.toml`

### Manual Configuration

Add the following snippets to your `languages.toml`:

```toml
[language-server.scheme-toolbox-lsp]
command = "scheme-toolbox-lsp"

[language-server.racket-langserver]
command = "racket"
args = ["-l", "racket-langserver"]

[[language]]
name = "racket"
scope = "source.rkt"
injection-regex = "racket"
file-types = ["rkt", "rktd", "rktl", "rktnb"]
shebangs = ["racket"]
comment-token = ";"
# Helix will use both servers and merge results
language-servers = [ "scheme-toolbox-lsp", "racket-langserver" ]

[[language]]
name = "scheme"
scope = "source.scm"
injection-regex = "scheme"
file-types = ["scm", "ss", "scmnb"]
shebangs = ["guile", "racket", "scheme"]
comment-token = ";"
language-servers = [ "scheme-toolbox-lsp" ]
```

### Removing racket-langserver

If you prefer to use _only_ `scheme-toolbox-lsp` (to avoid duplicate diagnostics or for performance), modify the `language-servers` list in your `languages.toml`:

```toml
[[language]]
name = "racket"
# ... other fields ...
language-servers = [ "scheme-toolbox-lsp" ]
```

## Comparison: scheme-toolbox-lsp vs. racket-langserver

| Feature          | scheme-toolbox-lsp                                                                         | racket-langserver                                          |
| :--------------- | :----------------------------------------------------------------------------------------- | :--------------------------------------------------------- |
| **Primary Goal** | **Evaluation & Feedback**                                                                  | **Static Analysis**                                        |
| **Inlay Hints**  | Live evaluation results (inline).                                                          | Type information / signatures.                             |
| **Diagnostics**  | Real-time evaluation and **syntax errors**.                                                | Syntax & static analysis errors.                           |
| **Performance**  | Optimized for fast REPL loops.                                                             | Can be slow on large files.                                |
| **Rich Media**   | Supported (Notebooks/Images).                                                              | Not supported.                                             |
| **Pros**         | Immediate visual feedback on code behavior; great for "The Little Schemer" style learning. | Deep understanding of Racket macro expansion and bindings. |
| **Cons**         | Syntax errors may overlap with racket-langserver.                                          | Heavy resource usage; no live evaluation hints.            |

**Recommendation**: Use both! Helix will merge the inlay hints and diagnostics, giving you the best of both worlds: live execution feedback from `scheme-toolbox-lsp` and deep static analysis from `racket-langserver`. (Note: You may see duplicate syntax errors until issue `ts-5v5` is implemented).

### Automation

You can use the provided `just` recipe to automatically append this configuration to your user `languages.toml`:

```bash
just configure-helix
```

## Notebook Support

`scheme-toolbox-lsp` supports a "notebook-style" evaluation loop using `.rktnb` (Racket Notebook) and `.scmnb` (Scheme Notebook) files.

- **File Format**: `.rktnb` and `.scmnb` files are **standard Racket/Scheme source files**. You can run them directly with the `racket` command-line tool. The extension is used by the LSP to enable specific block-parsing and evaluation behaviors.
- **Evaluation**: The LSP treats these files as a sequence of evaluable blocks.
    - **Block Separators**: Blocks are defined by **two or more newlines** (double newlines). This allows the LSP to provide granular inlay hints for each section of code.
    - **Markdown Blocks**: You can include documentation using `#| markdown ... |#` blocks. These are treated as non-code blocks by the evaluator but are preserved in the notebook structure.
- **Rich Media**: While Helix is a text-based editor and cannot render images/plots directly, the LSP still captures rich media results and logs them to the session log for external viewing.

## Features Supported

The `scheme-toolbox-lsp` provides the following features in Helix:

- **Inlay Hints**: Inline evaluation results for expressions.
- **Diagnostics**: Syntax errors and evaluation errors highlighted in the editor.
- **Code Actions**: Quick fixes for common issues.
- **Evaluation**: Commands to evaluate the current file or selection.
  - Note: Helix command integration for `scheme.evaluate` may require additional configuration or custom keybindings.

## Troubleshooting

Run `hx --health racket` to check if Helix correctly detects the language server.
