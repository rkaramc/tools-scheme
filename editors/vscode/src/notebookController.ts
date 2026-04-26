import * as vscode from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';

export class SchemeNotebookController {
    readonly controllerId = 'scheme-controller';
    readonly notebookType = 'scheme-notebook';
    readonly label = 'Scheme/Racket (LSP)';
    readonly supportedLanguages = ['racket', 'scheme', 'racket-notebook-cell', 'scheme-notebook-cell'];

    private readonly _controller: vscode.NotebookController;
    private _executionOrder = 0;
    
    // Map to keep track of active executions by cell URI string
    private _activeExecutions = new Map<string, vscode.NotebookCellExecution>();

    constructor(private readonly client: () => LanguageClient | undefined) {
        this._controller = vscode.notebooks.createNotebookController(
            this.controllerId,
            this.notebookType,
            this.label
        );

        this._controller.supportedLanguages = this.supportedLanguages;
        this._controller.supportsExecutionOrder = true;
        this._controller.executeHandler = this._execute.bind(this);
        this._controller.interruptHandler = this._interrupt.bind(this);
    }

    dispose() {
        this._controller.dispose();
    }

    private async _execute(
        cells: vscode.NotebookCell[],
        _notebook: vscode.NotebookDocument,
        _controller: vscode.NotebookController
    ): Promise<void> {
        for (const cell of cells) {
            await this._doExecution(cell);
        }
    }

    private async _doExecution(cell: vscode.NotebookCell): Promise<void> {
        const execution = this._controller.createNotebookCellExecution(cell);
        execution.executionOrder = ++this._executionOrder;
        execution.start(Date.now());
        execution.clearOutput();

        const c = this.client();
        if (!c) {
            execution.end(false, Date.now());
            vscode.window.showErrorMessage("LSP Client not running.");
            return;
        }

        const uriStr = cell.document.uri.toString();
        const notebookUriStr = cell.notebook.uri.toString();
        this._activeExecutions.set(uriStr, execution);

        // Send execution request to LSP
        try {
            await c.sendNotification('scheme/notebook/evalCell', {
                uri: uriStr,
                notebookUri: notebookUriStr,
                code: cell.document.getText(),
                executionId: execution.executionOrder,
                version: cell.document.version
            });
        } catch (err) {
            execution.appendOutput(new vscode.NotebookCellOutput([
                vscode.NotebookCellOutputItem.error(err as Error)
            ]));
            execution.end(false, Date.now());
            this._activeExecutions.delete(uriStr);
        }
    }

    private _interrupt(notebook: vscode.NotebookDocument): void {
        const c = this.client();
        if (!c) return;

        for (const cell of notebook.getCells()) {
            const uriStr = cell.document.uri.toString();
            const execution = this._activeExecutions.get(uriStr);
            if (execution) {
                c.sendNotification('scheme/notebook/cancelEval', {
                    uri: uriStr,
                    executionId: execution.executionOrder
                }).catch(err => {
                    console.error("Failed to send cancelEval:", err);
                });
            }
        }
    }

    /**
     * Handle incoming output streams from the LSP.
     */
    public async handleOutputStream(params: any): Promise<void> {
        const payload = params.payload;
        const executionId = params.executionId;

        // Find the execution matching this ID
        let targetExecution: vscode.NotebookCellExecution | undefined;
        for (const exec of this._activeExecutions.values()) {
            if (exec.executionOrder === executionId) {
                targetExecution = exec;
                break;
            }
        }

        if (!targetExecution) return;

        let outputItem: vscode.NotebookCellOutputItem | undefined;

        if (payload.type === 'stdout') {
            outputItem = vscode.NotebookCellOutputItem.stdout(payload.data + '\n');
        } else if (payload.type === 'stderr') {
            outputItem = vscode.NotebookCellOutputItem.stderr(payload.data + '\n');
        } else if (payload.type === 'result') {
            outputItem = vscode.NotebookCellOutputItem.text(payload.data);
        } else if (payload.type === 'rich') {
            try {
                // Determine correct mime type
                let mime = payload.mime || 'image/png';
                if (mime === 'image/png') {
                    const buf = Buffer.from(payload.data, 'base64');
                    outputItem = new vscode.NotebookCellOutputItem(buf, mime);
                } else {
                    // Fallback to text
                    outputItem = vscode.NotebookCellOutputItem.text(payload.data, mime);
                }
            } catch (err) {
                outputItem = vscode.NotebookCellOutputItem.error(err as Error);
            }
        } else if (payload.type === 'error') {
            outputItem = vscode.NotebookCellOutputItem.error({
                name: 'Evaluation Error',
                message: payload.data
            });
        }

        if (outputItem) {
            // Check if the last output was stdout, and append to it if so
            // VS Code appendOutput handles creating a new output block, but sometimes we want to merge stdout.
            // For simplicity, we just push a new output or let VS Code merge text internally.
            await targetExecution.appendOutput(new vscode.NotebookCellOutput([outputItem]));
        }
    }

    /**
     * Handle finished evaluation notification from the LSP.
     */
    public handleEvalFinished(params: any): void {
        const executionId = params.executionId;
        const success = params.success;

        // Find the execution matching this ID
        let targetUri: string | undefined;
        let targetExecution: vscode.NotebookCellExecution | undefined;

        for (const [uri, exec] of this._activeExecutions.entries()) {
            if (exec.executionOrder === executionId) {
                targetExecution = exec;
                targetUri = uri;
                break;
            }
        }

        if (targetExecution) {
            targetExecution.end(success, Date.now());
            if (targetUri) {
                this._activeExecutions.delete(targetUri);
            }
        }
    }
}
