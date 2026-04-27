# Standardized build and installation for Tools Scheme

set shell := ["powershell", "-command"]

# Level 1: Build in debug mode for host
# Level 2: Build in release mode for host
# Level 3: Build in release mode for multiple platforms
build level="1":
    @just _build-{{level}}

# Internal recipes for levels
_build-1:
    Write-Host ">>> Level 1: Debug Build (LSP + Extension)"
    cd lsp; cargo build
    cd editors/vscode; npm run compile

_build-2:
    Write-Host ">>> Level 2: Release Build (LSP + Extension)"
    cd lsp; cargo build --release
    cd editors/vscode; npm run compile

_build-3:
    Write-Host ">>> Level 3: Multi-platform Release Build"
    cd lsp; cargo build --release
    # Future: Add cross-compilation targets here
    cd editors/vscode; vsce package

# Build in debug mode (Quick)
debug:
    just build 1

# Build in release mode
release:
    just build 2

# Install the LSP and VS Code extension (Release builds only)
install:
    if (!(Test-Path "lsp/target/release/scheme-toolbox-lsp.exe") -and !(Test-Path "lsp/target/release/scheme-toolbox-lsp")) { \
        Write-Host "Error: Release build not found. Please run 'just build 2' first."; \
        exit 1; \
    }
    Write-Host "Installing LSP via cargo install..."
    cd lsp; cargo install --path .
    Write-Host "Installing VS Code extension..."
    cd editors/vscode; vsce package; \
    $vsix = Get-ChildItem -Filter *.vsix | Select-Object -First 1; \
    if ($vsix) { code --install-extension $vsix.FullName } else { Write-Host "Error: VSIX not found"; exit 1 }

# Publication orchestration and guide reference
publish:
    Write-Host "Note: Publication involves manual steps and registry setup."
    Write-Host "Please refer to publish_guide.md for the full checklist."
    # Add automated pre-checks here
    just test lsp
    just test vscode

# Run integration tests (requires VS Code to be installed)
integration-test:
    @Write-Host ">>> Running VSCode Integration Tests"
    cd editors/vscode; npm run test-integration > test-vscode-output.txt 2>&1

# Run tests for LSP or VS Code extension
# Examples:
#   just test lsp
#   just test vscode src/tests/utils.test.ts
#   just test lsp -- --nocapture
test target="all" *args:
    @if ("{{target}}" -eq "lsp") { \
        cd lsp; cargo test {{args}} \
    } elseif ("{{target}}" -eq "vscode") { \
        cd editors/vscode; npm test {{args}} > test-vscode-output.txt 2>&1 \
    } elseif ("{{target}}" -eq "all") { \
        Write-Host ">>> Testing LSP"; cd lsp; cargo test; \
        Write-Host ">>> Testing VSCode Extension"; cd editors/vscode; npm test > test-vscode-output.txt 2>&1 \
    } else { \
        Write-Host "Error: Unknown test target '{{target}}'. Use 'lsp', 'vscode', or 'all'."; \
        exit 1 \
    }
