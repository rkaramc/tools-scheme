# Spec: VS Code Extension for tools-scheme

## Objective
Create a VS Code extension that integrates the `scheme-toolbox-lsp` server into the editor. This extension will enable language support for Scheme and Racket, including the "Evaluate File" code action and inlay hints.

## Tech Stack
- **Language**: TypeScript
- **Framework**: `vscode-languageclient`
- **Build System**: `npm`

## Commands
- **Install Dependencies**: `npm install` (in `editors/vscode`)
- **Build Extension**: `npm run compile`
- **Package Extension**: `vsce package`

## Project Structure
```
editors/vscode/
├── package.json        → Extension manifest
├── tsconfig.json       → TypeScript configuration
├── src/
│   └── extension.ts    → Extension entry point
└── .vscode/
    └── launch.json     → Debug configurations
```

## Implementation Strategy
1. **Bootstrap**: Initialize a minimal VS Code extension project.
2. **LSP Integration**:
   - Define a `LanguageClient` that points to the `scheme-toolbox-lsp` binary.
   - Configure the client to handle `.rkt` and `.scm` files.
3. **Command Registration**:
   - Register the `scheme.evaluate` command in `package.json`.
   - Implement the command handler in `extension.ts` to delegate to the LSP's `workspace/executeCommand`.
4. **Activation**: The extension should activate when a Scheme or Racket file is opened.

## Code Style (TypeScript)
```typescript
import * as vscode from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions } from 'vscode-languageclient/node';

export function activate(context: vscode.ExtensionContext) {
    const serverOptions: ServerOptions = {
        command: "../../target/debug/scheme-toolbox-lsp",
        args: [],
    };
    // ...
}
```

## Testing Strategy
- **Manual Verification**: Launch the extension using the "Extension Development Host" in VS Code.
- **Integration**: Verify that opening a `.rkt` file starts the LSP and that "Evaluate Scheme File" appears in the context menu.

## Boundaries
- **Always**: Use absolute paths or reliable relative paths for the LSP binary.
- **Ask first**: Before adding complex UI components (e.g., custom sidebars).
- **Never**: Hardcode paths that are specific to a single user's machine.

## Success Criteria
- [x] VS Code extension project initialized in `editors/vscode`.
- [x] LSP server starts automatically when a `.rkt` file is opened. (Verified by extension activation events)
- [x] "Evaluate Scheme File" code action is available and functional.
- [x] Inlay hints are displayed after evaluation.
- [x] Diagnostics are displayed for errors.
