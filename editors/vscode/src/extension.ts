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

    // On Windows, copy the executable to a temporary location to avoid locking the original
    if (process.platform === 'win32') {
        try {
            const tempName = `scheme-toolbox-lsp-${Date.now()}.exe`;
            tempServerPath = path.join(os.tmpdir(), tempName);
            fs.copyFileSync(serverPath, tempServerPath);
            serverPath = tempServerPath;
            outputChannel.appendLine(`Copied LSP binary to temporary location: ${tempServerPath}`);
        } catch (err) {
            outputChannel.appendLine(`Failed to copy LSP binary to temporary location: ${err}`);
        }
    }

    // Determine the path to the Racket shim
    let shimPath = customShimPath;
    if (!shimPath && envLspDir) {
        const envPath = path.join(envLspDir, 'eval-shim.rkt');
        if (require('fs').existsSync(envPath)) {
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

    outputChannel.appendLine(`LSP Server Path: ${serverPath}`);
    outputChannel.appendLine(`Racket Shim Path: ${shimPath}`);

    const serverOptions: ServerOptions = {
        command: serverPath,
        args: [shimPath], // Pass shim path as first argument
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

    context.subscriptions.push(evaluateCommand);

    // Start the client
    outputChannel.appendLine('Starting LSP client...');
    client.start();
}

export async function deactivate(): Promise<void> {
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
