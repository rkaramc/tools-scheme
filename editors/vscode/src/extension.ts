import * as vscode from 'vscode';
import * as path from 'path';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient;
let outputChannel: vscode.OutputChannel;

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
        if (require('fs').existsSync(envPath)) {
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
            provideInlineValues: async (document, viewPort, context, token, next) => {
                outputChannel.appendLine(`[InlineValues] Requesting values for ${document.uri.toString()}`);
                const result = await next(document, viewPort, context, token);
                outputChannel.appendLine(`[InlineValues] Received values: ${JSON.stringify(result, null, 2)}`);
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

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}
