# Standardized build and installation for Tools Scheme

set quiet := true
set shell := ["bash", "-c"]
set windows-shell := ["pwsh", "-NoProfile", "-Command"]

# Variables
os := os()
exe := if os == "windows" { ".exe" } else { "" }
lsp_binary := "target/release/scheme-toolbox-lsp" + exe
tmp_dir_base := justfile_directory() + "/tmp"
tmp_dir := if os == "windows" { replace(tmp_dir_base, "/", "\\") } else { tmp_dir_base }

export TOOLS_SCHEME_TMP_DIR := tmp_dir

# Ensure tmp directory exists and rotate previous logs
_ensure-tmp:
    {{ if os == "windows" { "if (!(Test-Path '" + tmp_dir + "')) { $null = New-Item -ItemType Directory -Path '" + tmp_dir + "' }; if (Test-Path '" + tmp_dir + "\\test-output.txt') { Move-Item -Path '" + tmp_dir + "\\test-output.txt' -Destination '" + tmp_dir + "\\test-output.prev.txt' -Force }" } else { "mkdir -p '" + tmp_dir + "'; if [ -f '" + tmp_dir + "/test-output.txt' ]; then mv -f '" + tmp_dir + "/test-output.txt' '" + tmp_dir + "/test-output.prev.txt'; fi" } }}

# Level 1: Build in debug mode for host
# Level 2: Build in release mode for host
# Level 3: Build in release mode for multiple platforms
build level="1":
    @just _build-{{level}}

# Internal recipes for levels
_build-1:
    echo ">>> Level 1: Debug Build (LSP + Extension)"
    cd lsp && cargo build
    cd editors/vscode && npm run compile

_build-2:
    echo ">>> Level 2: Release Build (LSP + Extension)"
    cd lsp && cargo build --release
    cd editors/vscode && vsce package

_build-3:
    echo ">>> Level 3: Multi-platform Release Build"
    cd lsp && cargo build --release
    # Future: Add cross-compilation targets here
    cd editors/vscode && vsce package

# Build in debug mode (Quick)
debug:
    just build 1

# Build in release mode
release:
    just build 2

# Install the LSP and VS Code extension (Release builds only)
[unix]
install: release
    echo "Installing LSP via cargo install..."
    cd lsp && cargo install --path .
    echo "Installing VS Code extension..."
    NAME=$(jq -r .name editors/vscode/package.json)
    VERSION=$(jq -r .version editors/vscode/package.json)
    VSIX="editors/vscode/$NAME-$VERSION.vsix"
    if [ -f "$VSIX" ]; then \
        echo "Installing extension: $VSIX"; \
        code --install-extension "$VSIX"; \
    else \
        echo "Error: VSIX not found ($VSIX)"; \
        exit 1; \
    fi

[windows]
install: release
    echo "Installing LSP via cargo install..."
    cd lsp; cargo install --path .
    echo "Installing VS Code extension..."
    $json = Get-Content editors/vscode/package.json | ConvertFrom-Json
    $vsix = "editors/vscode/$($json.name)-$($json.version).vsix"
    if (Test-Path $vsix) { \
        echo "Installing extension: $vsix"; \
        code --install-extension $vsix; \
    } else { \
        Write-Error "Error: VSIX not found ($vsix)"; \
        exit 1; \
    }

# Publication orchestration and guide reference
publish:
    echo "Note: Publication involves manual steps and registry setup."
    echo "Please refer to publish_guide.md for the full checklist."
    # Add automated pre-checks here
    just test lsp
    just test vscode

# Run tests for LSP, VS Code extension, or eval-shim
test target="all" *args:
    @just _test-{{target}} {{args}}

# Run all test suites
_test-all *args:
    @just clean
    @just debug
    @just _test-lsp {{args}}
    @just _test-vscode {{args}}
    @just _test-racket {{args}}
    @just _test-integration -- --test-threads=1 {{args}}

# Run tests for LSP (Rust)
_test-lsp *args: _ensure-tmp
    echo ">>> Testing LSP (Rust+Cargo)"
    cd lsp && cargo test {{args}} -- --test-threads=1 >> {{tmp_dir}}/test-output.txt 2>&1

# Run specifically integration tests for LSP
test-lsp-integration *args: _ensure-tmp
    echo ">>> Testing LSP Integration (Sequential)"
    cd lsp && cargo test --test integration -- --test-threads=1 {{args}} >> {{tmp_dir}}/test-output.txt 2>&1

# Run specifically windows tests for LSP
test-lsp-windows *args: _ensure-tmp
    echo ">>> Testing LSP Windows Specifics"
    cd lsp && cargo test --test windows_test {{args}} >> {{tmp_dir}}/test-output.txt 2>&1

# Run tests for VSCode extension (Typescript+Jest)
_test-vscode *args: _ensure-tmp
    echo ">>> Testing VSCode Extension (TypeScript)"
    cd editors/vscode && npm test {{args}} >> {{tmp_dir}}/test-output.txt 2>&1

# Run tests for Racket eval-shim (Racket+RacoTest)
_test-racket *args: _ensure-tmp
    echo ">>> Testing Eval-shim (Racket)"
    raco test lsp/src/eval-shim.rkt {{args}} >> {{tmp_dir}}/test-output.txt 2>&1

# Run integrations tests on VSCode (VSCode + extension host + Mocha) (requires VS Code to be installed)
_test-integration *args: _ensure-tmp
    echo ">>> Testing Integration with VS Code"
    cd editors/vscode && npm run test-integration {{args}} >> {{tmp_dir}}/test-output.txt 2>&1

# Clean project outputs
clean:
    @just _clean-test-outputs
    cargo clean

# Clean test outputs
_clean-test-outputs:
    echo ">>> Clean test outputs"
    {{ if os == "windows" { "if (Test-Path '" + tmp_dir + "') { Get-ChildItem -Path '" + tmp_dir + "' -Exclude 'test-output*.txt' | Remove-Item -Recurse -Force }" } else { "find '" + tmp_dir + "' -mindepth 1 ! -name 'test-output*.txt' -exec rm -rf {} +" } }}
