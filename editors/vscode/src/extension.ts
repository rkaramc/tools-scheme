import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";
import {
  resolveLspPath,
  cleanupStaleFiles,
  getRuntimeBinaryPath,
} from "./utils";

let client: LanguageClient;
let outputChannel: vscode.OutputChannel;
let tempServerPath: string | undefined;
let originalServerPath: string | undefined;
let lspWatcher: fs.FSWatcher | undefined;
import * as fs from "fs"; // Needed for fs.watch and other file ops
import * as path from "path";

export function activate(context: vscode.ExtensionContext) {
  outputChannel = vscode.window.createOutputChannel("Scheme Toolbox");
  outputChannel.appendLine("Activating Scheme Toolbox extension...");

  // 1. Resolve LSP binary path
  const lspPath = resolveLspPath(context);
  if (!lspPath) {
    const msg =
      'Scheme Toolbox: Could not find "scheme-toolbox-lsp" binary.' +
      'Please install it on your PATH or set "scheme.lspPath" in settings or set "TOOLS_SCHEME_LSP_PATH" environment variable.';
    outputChannel.appendLine(msg);
    vscode.window.showErrorMessage(msg);
    return;
  }
  originalServerPath = lspPath;

  // 2. Prepare runtime binary (Windows lock workaround in development)
  const isDevelopment =
    context.extensionMode === vscode.ExtensionMode.Development;
  if (isDevelopment) {
    cleanupStaleFiles(outputChannel);

    // Watch the original LSP binary for changes (Development only)
    let watchTimeout: NodeJS.Timeout | undefined;
    try {
      lspWatcher = fs.watch(originalServerPath, (event) => {
        if (event === "change") {
          if (watchTimeout) {
            clearTimeout(watchTimeout);
          }
          watchTimeout = setTimeout(() => {
            outputChannel.appendLine(
              `Detected change in LSP binary: ${originalServerPath}. Restarting...`,
            );
            restartClient(context);
          }, 500);
        }
      });
    } catch (err) {
      outputChannel.appendLine(
        `Failed to start file watcher for LSP binary: ${err}`,
      );
    }
  }

  startClient(context);

  // Register the custom command that delegates to the LSP
  const evaluateCommand = vscode.commands.registerCommand(
    "scheme.runEvaluation",
    async (uriOrArgs: any) => {
      outputChannel.appendLine(
        `Triggering evaluation for: ${JSON.stringify(uriOrArgs)}`,
      );

      let uri: string;
      if (typeof uriOrArgs === "string") {
        uri = uriOrArgs;
      } else if (uriOrArgs instanceof vscode.Uri) {
        uri = uriOrArgs.toString();
      } else {
        const activeEditor = vscode.window.activeTextEditor;
        if (activeEditor) {
          uri = activeEditor.document.uri.toString();
        } else {
          vscode.window.showErrorMessage("No active editor to evaluate.");
          return;
        }
      }

      if (!client) {
        vscode.window.showErrorMessage("LSP Client not initialized.");
        return;
      }

      try {
        const result = await client.sendRequest("workspace/executeCommand", {
          command: "scheme.evaluate",
          arguments: [uri],
        });
        outputChannel.appendLine(
          `Evaluation command completed. Results:\n${JSON.stringify(result, null, 2)}`,
        );
      } catch (err) {
        outputChannel.appendLine(`Evaluation failed: ${err}`);
        vscode.window.showErrorMessage(`Evaluation failed: ${err}`);
      }
    },
  );

  const evaluateSelectionCommand = vscode.commands.registerCommand(
    "scheme.runEvaluateSelection",
    async () => {
      const activeEditor = vscode.window.activeTextEditor;
      if (!activeEditor) {
        vscode.window.showErrorMessage(
          "No active editor to evaluate selection from.",
        );
        return;
      }

      const selection = activeEditor.selection;
      if (selection.isEmpty) {
        vscode.window.showInformationMessage("No text selected to evaluate.");
        return;
      }

      const selectedText = activeEditor.document.getText(selection);
      const uri = activeEditor.document.uri.toString();

      outputChannel.appendLine(`Triggering selection evaluation for: ${uri}`);

      if (!client) {
        vscode.window.showErrorMessage("LSP Client not initialized.");
        return;
      }

      try {
        const result = await client.sendRequest("workspace/executeCommand", {
          command: "scheme.evaluateSelection",
          arguments: [
            uri,
            selectedText,
            {
              start: selection.start,
              end: selection.end,
            },
          ],
        });
        outputChannel.appendLine(
          `Evaluate selection command completed. Results:\n${JSON.stringify(result, null, 2)}`,
        );
      } catch (err) {
        outputChannel.appendLine(`Evaluate selection failed: ${err}`);
        vscode.window.showErrorMessage(`Evaluate selection failed: ${err}`);
      }
    },
  );

  const restartREPLCommand = vscode.commands.registerCommand(
    "scheme.restartREPL",
    async () => {
      if (!client) {
        vscode.window.showErrorMessage("LSP Client not initialized.");
        return;
      }

      try {
        await client.sendRequest("workspace/executeCommand", {
          command: "scheme.restartREPL",
          arguments: [],
        });
        vscode.window.showInformationMessage("Racket restarted.");
      } catch (err) {
        outputChannel.appendLine(`Restart Racket failed: ${err}`);
        vscode.window.showErrorMessage(`Failed to restart Racket: ${err}`);
      }
    },
  );

  const clearNamespaceCommand = vscode.commands.registerCommand(
    "scheme.clearNamespace",
    async () => {
      const activeEditor = vscode.window.activeTextEditor;
      if (!activeEditor) {
        vscode.window.showErrorMessage("No active editor to reset file for.");
        return;
      }

      const uri = activeEditor.document.uri.toString();

      if (!client) {
        vscode.window.showErrorMessage("LSP Client not initialized.");
        return;
      }

      try {
        await client.sendRequest("workspace/executeCommand", {
          command: "scheme.clearNamespace",
          arguments: [uri],
        });
        vscode.window.showInformationMessage("File reset.");
      } catch (err) {
        outputChannel.appendLine(`Reset File failed: ${err}`);
        vscode.window.showErrorMessage(`Failed to reset file: ${err}`);
      }
    },
  );

  context.subscriptions.push(evaluateCommand);
  context.subscriptions.push(evaluateSelectionCommand);
  context.subscriptions.push(restartREPLCommand);
  context.subscriptions.push(clearNamespaceCommand);

  // Handle custom file extensions
  const handleCustomExtensions = () => {
    const config = vscode.workspace.getConfiguration("scheme");
    const customExts =
      config.get<Record<string, string>>("customFileExtensions") || {};

    const apply = (doc: vscode.TextDocument) => {
      const ext = path.extname(doc.uri.fsPath);
      const targetLang = customExts[ext];
      if (targetLang && doc.languageId !== targetLang) {
        vscode.languages.setTextDocumentLanguage(doc, targetLang);
      }
    };

    vscode.workspace.textDocuments.forEach(apply);
    return vscode.workspace.onDidOpenTextDocument(apply);
  };

  context.subscriptions.push(handleCustomExtensions());

  // Watch for configuration changes to restart client if extensions change
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("scheme.customFileExtensions")) {
        outputChannel.appendLine(
          "Custom file extensions changed. Restarting LSP client...",
        );
        restartClient(context);
      }
    }),
  );
}

function startClient(context: vscode.ExtensionContext) {
  if (!originalServerPath) {
    return;
  }
  const { newPath: serverPath, updatedTempPath } = getRuntimeBinaryPath(context, originalServerPath, outputChannel, tempServerPath);
  tempServerPath = updatedTempPath || tempServerPath;

  outputChannel.appendLine(`LSP Server Path: ${serverPath}`);

  const serverOptions: ServerOptions = {
    command: serverPath,
    args: [],
  };

  const config = vscode.workspace.getConfiguration("scheme");
  const customExts =
    config.get<Record<string, string>>("customFileExtensions") || {};
  const defaultExts = ["rkt", "scm", "ss"];
  const allExts = [
    ...defaultExts,
    ...Object.keys(customExts).map((ext) =>
      ext.startsWith(".") ? ext.slice(1) : ext,
    ),
  ];
  const uniqueExts = Array.from(new Set(allExts));
  const extGlob =
    uniqueExts.length > 1 ? `{${uniqueExts.join(",")}}` : uniqueExts[0];

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "racket" },
      { scheme: "file", language: "scheme" },
    ],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher(`**/*.${extGlob}`),
    },
    initializationOptions: {
      racketPath: config.get<string>("racketPath"),
    },
    outputChannel: outputChannel,
    middleware: {
      provideInlayHints: async (document, range, token, next) => {
        const result = await next(document, range, token);
        outputChannel.appendLine(
          `[InlayHints] Received ${result?.length} hints for ${document.uri.toString()} over range: ${JSON.stringify(range)}`,
        );
        return result;
      },
    },
  };

  client = new LanguageClient(
    "schemeToolboxLsp",
    "Scheme Toolbox LSP",
    serverOptions,
    clientOptions,
  );

  // Start the client
  outputChannel.appendLine("Starting LSP client...");
  client.start();
}

async function restartClient(context: vscode.ExtensionContext) {
  if (!originalServerPath) {
    return;
  }
  // 1. Stop old client
  const oldTempPath = tempServerPath;
  if (client) {
    outputChannel.appendLine("Stopping old LSP client...");
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
