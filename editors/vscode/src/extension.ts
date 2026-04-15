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
let originalServerPath: string;
let currentShimPath: string;
let lspWatcher: fs.FSWatcher | undefined;

export function activate(context: vscode.ExtensionContext) {
    outputChannel = vscode.window.createOutputChannel('Scheme Toolbox');
    outputChannel.appendLine('Activating Scheme Toolbox extension...');

    const config = vscode.workspace.getConfiguration('scheme');
    const customLspPath = config.get<string>('lspPath');
    const customShimPath = config.get<string>('shimPath');
    const envLspDir = process.env.TOOLS_SCHEME_LSP_PATH;

    // Determine the path to the LSP binary
    let serverPath = customLspPath;
    if (!serverPath && envLspDir) {
        const binName = process.platform === 'win32' ? 'scheme-toolbox-lsp.exe' : 'scheme-toolbox-lsp';
        const envPath = path.join(envLspDir, binName);
        if (fs.existsSync(envPath)) {
            serverPath = envPath;
        }
    }
    if (!serverPath) {
        serverPath = context.asAbsolutePath(path.join(
            '..',
            '..',
            'target',
            'debug',
            process.platform === 'win32' ? 'scheme-toolbox-lsp.exe' : 'scheme-toolbox-lsp'
        ));
    }

    originalServerPath = serverPath;

    // Determine the path to the Racket shim
    let shimPath = customShimPath;
    if (!shimPath && envLspDir) {
        const envPath = path.join(envLspDir, 'eval-shim.rkt');
        if (fs.existsSync(envPath)) {
            shimPath = envPath;
        }
    }
    if (!shimPath) {
        shimPath = context.asAbsolutePath(path.join(
            '..',
            '..',
            'lsp',
            'src',
            'eval-shim.rkt'
        ));
    }
    currentShimPath = shimPath;

    startClient(context);

    // Watch the original LSP binary for changes
    let watchTimeout: NodeJS.Timeout | undefined;
    try {
        lspWatcher = fs.watch(originalServerPath, (event) => {
            if (event === 'change') {
                if (watchTimeout) {
                    clearTimeout(watchTimeout);
                }
                watchTimeout = setTimeout(() => {
                    outputChannel.appendLine(`Detected change in LSP binary: ${originalServerPath}. Restarting...`);
                    restartClient();
                }, 500);
            }
        });
    } catch (err) {
        outputChannel.appendLine(`Failed to start file watcher for LSP binary: ${err}`);
    }


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
    let serverPath = originalServerPath;

    // On Windows, copy the executable to a temporary location to avoid locking the original
    if (process.platform === 'win32') {
        try {
            const tempName = `scheme-toolbox-lsp-${Date.now()}.exe`;
            const newTempPath = path.join(os.tmpdir(), tempName);
            fs.copyFileSync(originalServerPath, newTempPath);
            
            // Cleanup old temp file if it exists
            if (tempServerPath && fs.existsSync(tempServerPath)) {
                try { fs.unlinkSync(tempServerPath); } catch (e) {}
            }
            
            tempServerPath = newTempPath;
            serverPath = tempServerPath;
            outputChannel.appendLine(`Copied LSP binary to temporary location: ${tempServerPath}`);
        } catch (err) {
            outputChannel.appendLine(`Failed to copy LSP binary to temporary location: ${err}`);
        }
    }

    outputChannel.appendLine(`LSP Server Path: ${serverPath}`);
    outputChannel.appendLine(`Racket Shim Path: ${currentShimPath}`);

    const serverOptions: ServerOptions = {
        command: serverPath,
        args: [currentShimPath], // Pass shim path as first argument
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

async function restartClient() {
    outputChannel.appendLine('Restarting LSP client...');
    
    // 1. Prepare new binary (on Windows)
    let newServerPath = originalServerPath;
    let newTempPath: string | undefined;

    if (process.platform === 'win32') {
        try {
            const tempName = `scheme-toolbox-lsp-${Date.now()}.exe`;
            newTempPath = path.join(os.tmpdir(), tempName);
            fs.copyFileSync(originalServerPath, newTempPath);
            newServerPath = newTempPath;
            outputChannel.appendLine(`Copied new LSP binary to: ${newTempPath}`);
        } catch (err) {
            outputChannel.appendLine(`Failed to copy new LSP binary, will try to reuse existing: ${err}`);
        }
    }

    // 2. Stop old client
    const oldTempPath = tempServerPath;
    if (client) {
        outputChannel.appendLine('Stopping old LSP client...');
        await client.stop();
    }

    // 3. Update paths and start new client
    tempServerPath = newTempPath;
    
    // We need to re-initialize the client with new serverOptions
    // Note: In some versions of vscode-languageclient, you can just update serverOptions.
    // But re-creating the client is safer for path changes.
    const serverOptions: ServerOptions = {
        command: newServerPath,
        args: [currentShimPath],
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

    outputChannel.appendLine('Starting new LSP client...');
    await client.start();

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
