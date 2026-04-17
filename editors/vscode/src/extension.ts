import * as vscode from 'vscode';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient;
let outputChannel: vscode.OutputChannel;
let tempServerPath: string | undefined;
let originalServerPath: string | undefined;
let currentShimPath: string | undefined;
let lspWatcher: fs.FSWatcher | undefined;

const TEMP_DIR_NAME = 'vscode-scheme-toolbox-lsp';

/**
 * Returns the path to the dedicated temporary directory for LSP binaries.
 */
function getTempDir(): string {
    const tempDir = path.join(os.tmpdir(), TEMP_DIR_NAME);
    if (!fs.existsSync(tempDir)) {
        fs.mkdirSync(tempDir, { recursive: true });
    }
    return tempDir;
}

/**
 * Attempts to clean up stale temporary LSP binaries from previous sessions.
 */
function cleanupStaleFiles() {
    const tempDir = path.join(os.tmpdir(), TEMP_DIR_NAME);
    if (!fs.existsSync(tempDir)) { return; }

    try {
        const files = fs.readdirSync(tempDir);
        for (const file of files) {
            if (file.startsWith('scheme-toolbox-lsp-') && file.endsWith('.exe')) {
                const filePath = path.join(tempDir, file);
                try {
                    fs.unlinkSync(filePath);
                    outputChannel.appendLine(`Cleaned up stale temporary binary: ${file}`);
                } catch (err) {
                    // File is likely in use by another VS Code instance, ignore
                }
            }
        }
    } catch (err) {
        outputChannel.appendLine(`Failed to scan temporary directory for cleanup: ${err}`);
    }
}

/**
 * Searches for a binary in the system PATH.
 */
function findInPath(binaryName: string): string | undefined {
    const paths = (process.env.PATH || process.env.Path || '').split(path.delimiter);
    for (const p of paths) {
        const fullPath = path.join(p, binaryName);
        if (fs.existsSync(fullPath)) {
            return fullPath;
        }
    }
    return undefined;
}

/**
 * Resolves the path to the LSP binary, checking settings, environment, PATH, and development fallback.
 */
function resolveLspPath(context: vscode.ExtensionContext): string | undefined {
    const config = vscode.workspace.getConfiguration('scheme');
    const customLspPath = config.get<string>('lspPath');
    const envLspDir = process.env.TOOLS_SCHEME_LSP_PATH;
    const binName = process.platform === 'win32' ? 'scheme-toolbox-lsp.exe' : 'scheme-toolbox-lsp';

    let serverPath = customLspPath;

    if (!serverPath && envLspDir) {
        const envPath = path.join(envLspDir, binName);
        if (fs.existsSync(envPath)) {
            serverPath = envPath;
        }
    }

    if (!serverPath) {
        serverPath = findInPath(binName);
    }

    if (!serverPath && context.extensionMode === vscode.ExtensionMode.Development) {
        const devPath = context.asAbsolutePath(path.join('..', '..', 'target', 'debug', binName));
        if (fs.existsSync(devPath)) {
            serverPath = devPath;
        }
    }

    return serverPath;
}

/**
 * Resolves the path to the Racket evaluation shim.
 */
function resolveShimPath(context: vscode.ExtensionContext, lspPath: string | undefined): string | undefined {
    const config = vscode.workspace.getConfiguration('scheme');
    const customShimPath = config.get<string>('shimPath');
    const envLspDir = process.env.TOOLS_SCHEME_LSP_PATH;

    let shimPath = customShimPath;
    if (!shimPath && envLspDir) {
        const envPath = path.join(envLspDir, 'eval-shim.rkt');
        if (fs.existsSync(envPath)) {
            shimPath = envPath;
        }
    }

    if (!shimPath && context.extensionMode === vscode.ExtensionMode.Development) {
        const devPath = context.asAbsolutePath(path.join('..', '..', 'lsp', 'src', 'eval-shim.rkt'));
        if (fs.existsSync(devPath)) {
            shimPath = devPath;
        }
    }

    // Default to the same directory as the LSP if not found elsewhere (standard install layout)
    if (!shimPath && lspPath) {
        const binDir = path.dirname(lspPath);
        const localShim = path.join(binDir, 'eval-shim.rkt');
        if (fs.existsSync(localShim)) {
            shimPath = localShim;
        }
    }

    return shimPath;
}

/**
 * Prepares the binary for execution. On Windows in Development mode, this involves
 * copying the binary to a temporary location to avoid locking.
 */
function getRuntimeBinaryPath(context: vscode.ExtensionContext, originalPath: string): string {
    const isDevelopment = context.extensionMode === vscode.ExtensionMode.Development;
    if (!isDevelopment || process.platform !== 'win32') {
        return originalPath;
    }

    try {
        const tempName = `scheme-toolbox-lsp-${Date.now()}.exe`;
        const newTempPath = path.join(getTempDir(), tempName);
        fs.copyFileSync(originalPath, newTempPath);

        // Cleanup old temp file if it exists
        if (tempServerPath && fs.existsSync(tempServerPath)) {
            try {
                fs.unlinkSync(tempServerPath);
            } catch (e) { }
        }

        tempServerPath = newTempPath;
        outputChannel.appendLine(`Copied LSP binary to temporary location: ${tempServerPath}`);
        return tempServerPath;
    } catch (err) {
        outputChannel.appendLine(`Failed to copy LSP binary to temporary location: ${err}`);
        return originalPath;
    }
}

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('Scheme Toolbox');
    outputChannel.appendLine('Activating Scheme Toolbox extension...');

    // 1. Resolve LSP binary path
    const lspPath = resolveLspPath(context);
    if (!lspPath) {
        const msg = 'Scheme Toolbox: Could not find "scheme-toolbox-lsp" binary. Please install it on your PATH or set "scheme.lspPath" in settings.';
        outputChannel.appendLine(msg);
        vscode.window.showErrorMessage(msg);
        return;
    }
    originalServerPath = lspPath;

    // 2. Resolve Racket shim path
    currentShimPath = resolveShimPath(context, originalServerPath);
    if (!currentShimPath) {
        outputChannel.appendLine('Scheme Toolbox: Could not find "eval-shim.rkt". Please set "scheme.shimPath" in settings.');
    }

    // 3. Prepare runtime binary (Windows lock workaround in development)
    const isDevelopment = context.extensionMode === vscode.ExtensionMode.Development;
    if (isDevelopment) {
        cleanupStaleFiles();

        // Watch the original LSP binary for changes (Development only)
        let watchTimeout: NodeJS.Timeout | undefined;
        try {
            lspWatcher = fs.watch(originalServerPath, (event) => {
                if (event === "change") {
                    if (watchTimeout) {
                        clearTimeout(watchTimeout);
                    }
                    watchTimeout = setTimeout(() => {
                        outputChannel.appendLine(`Detected change in LSP binary: ${originalServerPath}. Restarting...`);
                        restartClient(context);
                    }, 500);
                }
            });
        } catch (err) {
            outputChannel.appendLine(`Failed to start file watcher for LSP binary: ${err}`);
        }
    }

    startClient(context);

    // Register the custom command that delegates to the LSP
    const evaluateCommand = vscode.commands.registerCommand('scheme.runEvaluation', async (uriOrArgs: any) => {
        outputChannel.appendLine(`Triggering evaluation for: ${JSON.stringify(uriOrArgs)}`);
        
        let uri: string;
        if (typeof uriOrArgs === 'string') {
            uri = uriOrArgs;
        } else if (uriOrArgs instanceof vscode.Uri) {
            uri = uriOrArgs.toString();
        } else {
            const activeEditor = vscode.window.activeTextEditor;
            if (activeEditor) {
                uri = activeEditor.document.uri.toString();
            } else {
                vscode.window.showErrorMessage('No active editor to evaluate.');
                return;
            }
        }

        if (!client) {
            vscode.window.showErrorMessage('LSP Client not initialized.');
            return;
        }

        try {
            const result = await client.sendRequest('workspace/executeCommand', {
                command: 'scheme.evaluate',
                arguments: [uri]
            });
            outputChannel.appendLine(`Evaluation command completed. Results:\n${JSON.stringify(result, null, 2)}`);
        } catch (err) {
            outputChannel.appendLine(`Evaluation failed: ${err}`);
            vscode.window.showErrorMessage(`Evaluation failed: ${err}`);
        }
    });

    const evaluateSelectionCommand = vscode.commands.registerCommand('scheme.runEvaluateSelection', async () => {
        const activeEditor = vscode.window.activeTextEditor;
        if (!activeEditor) {
            vscode.window.showErrorMessage('No active editor to evaluate selection from.');
            return;
        }

        const selection = activeEditor.selection;
        if (selection.isEmpty) {
            vscode.window.showInformationMessage('No text selected to evaluate.');
            return;
        }

        const selectedText = activeEditor.document.getText(selection);
        const uri = activeEditor.document.uri.toString();

        outputChannel.appendLine(`Triggering selection evaluation for: ${uri}`);

        if (!client) {
            vscode.window.showErrorMessage('LSP Client not initialized.');
            return;
        }

        try {
            const result = await client.sendRequest('workspace/executeCommand', {
                command: 'scheme.evaluateSelection',
                arguments: [uri, selectedText]
            });
            outputChannel.appendLine(`Evaluate selection command completed. Results:\n${JSON.stringify(result, null, 2)}`);
        } catch (err) {
            outputChannel.appendLine(`Evaluate selection failed: ${err}`);
            vscode.window.showErrorMessage(`Evaluate selection failed: ${err}`);
        }
    });

    context.subscriptions.push(evaluateCommand);
    context.subscriptions.push(evaluateSelectionCommand);
}

function startClient(context: vscode.ExtensionContext) {
    if (!originalServerPath) { return; }
    const serverPath = getRuntimeBinaryPath(context, originalServerPath);

    outputChannel.appendLine(`LSP Server Path: ${serverPath}`);
    outputChannel.appendLine(`Racket Shim Path: ${currentShimPath}`);

    const serverOptions: ServerOptions = {
        command: serverPath,
        args: currentShimPath ? [currentShimPath] : [],
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'racket' },
            { scheme: 'file', language: 'scheme' },
        ],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.{rkt,scm,ss}'),
        },
        outputChannel: outputChannel,
        middleware: {
            provideInlayHints: async (document, range, token, next) => {
                outputChannel.appendLine(`[InlayHints] Requesting hints for ${document.uri.toString()} over range: ${JSON.stringify(range)}`);
                const result = await next(document, range, token);
                outputChannel.appendLine(`[InlayHints] Received hints: ${JSON.stringify(result, null, 2)}`);
                return result;
            }
        }
    };

    client = new LanguageClient(
        'schemeToolboxLsp',
        'Scheme Toolbox LSP',
        serverOptions,
        clientOptions
    );

    // Start the client
    outputChannel.appendLine('Starting LSP client...');
    client.start();
}

async function restartClient(context: vscode.ExtensionContext) {
    if (!originalServerPath) { return; }
    // 1. Stop old client
    const oldTempPath = tempServerPath;
    if (client) {
        outputChannel.appendLine('Stopping old LSP client...');
        await client.stop();
    }

    // 2. Start new client (getRuntimeBinaryPath handles Windows temp copying)
    startClient(context);

    // 4. Cleanup old temp file
    if (oldTempPath && fs.existsSync(oldTempPath)) {
        try {
            fs.unlinkSync(oldTempPath);
            outputChannel.appendLine(`Deleted old temporary binary: ${oldTempPath}`);
        } catch (err) {
            // Best effort
        }
    }
}

export async function deactivate(): Promise<void> {
    if (lspWatcher) {
        lspWatcher.close();
    }
    if (client) {
        await client.stop();
    }
    if (tempServerPath && fs.existsSync(tempServerPath)) {
        try {
            fs.unlinkSync(tempServerPath);
        } catch (err) {
            // Best effort cleanup
        }
    }
}
