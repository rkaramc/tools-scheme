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
        const executionId = params.executionId;
        const targetExecution = this._findExecution(executionId);
        if (!targetExecution) return;

        const payload = params.payload;
        if (payload.type === 'stdout' || payload.type === 'stderr') {
            await this._appendStreamOutput(targetExecution, payload);
            return;
        }

        const outputItem = await this._createOutputItem(payload);
        if (outputItem) {
            await targetExecution.appendOutput(new vscode.NotebookCellOutput([outputItem]));
        }
    }

    private _findExecution(executionId: number): vscode.NotebookCellExecution | undefined {
        for (const exec of this._activeExecutions.values()) {
            if (exec.executionOrder === executionId) {
                return exec;
            }
        }
        return undefined;
    }

    private async _appendStreamOutput(execution: vscode.NotebookCellExecution, payload: any): Promise<void> {
        const isStdout = payload.type === 'stdout';
        const mime = isStdout ? 'application/vnd.code.notebook.stdout' : 'application/vnd.code.notebook.stderr';

        const cell = execution.cell;
        const outputs = cell.outputs;
        const lastOutput = outputs.length > 0 ? outputs[outputs.length - 1] : undefined;

        if (lastOutput) {
            const existingItem = lastOutput.items.find(item => item.mime === mime);
            if (existingItem) {
                const currentData = Buffer.from(existingItem.data).toString('utf8');
                const newData = currentData + payload.data;
                const newItem = isStdout 
                    ? vscode.NotebookCellOutputItem.stdout(newData) 
                    : vscode.NotebookCellOutputItem.stderr(newData);
                
                const newItems = lastOutput.items.map(item => item.mime === mime ? newItem : item);
                await execution.replaceOutputItems(newItems, lastOutput);
                return;
            }
        }

        const item = isStdout 
            ? vscode.NotebookCellOutputItem.stdout(payload.data) 
            : vscode.NotebookCellOutputItem.stderr(payload.data);
        await execution.appendOutput(new vscode.NotebookCellOutput([item]));
    }

    private async _createOutputItem(payload: any): Promise<vscode.NotebookCellOutputItem | undefined> {
        if (payload.type === 'result') {
            return vscode.NotebookCellOutputItem.text(payload.data);
        }

        if (payload.type === 'rich') {
            return this._handleRichPayload(payload);
        }

        if (payload.type === 'error') {
            return vscode.NotebookCellOutputItem.error({
                name: 'Evaluation Error',
                message: payload.data
            });
        }

        return undefined;
    }

    private async _handleRichPayload(payload: any): Promise<vscode.NotebookCellOutputItem | undefined> {
        try {
            const mime = payload.mime || 'image/png';
            let data = payload.data;

            if (payload.id) {
                const c = this.client();
                if (c) {
                    const response = await c.sendRequest<{ data: string }>('scheme/notebook/pullRichMedia', { id: payload.id });
                    data = response.data;
                }
            }

            if (!data) return undefined;

            if (mime === 'image/png') {
                const buf = Buffer.from(data, 'base64');
                return new vscode.NotebookCellOutputItem(buf, mime);
            }
            
            return vscode.NotebookCellOutputItem.text(data, mime);
        } catch (err) {
            return vscode.NotebookCellOutputItem.error(err as Error);
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
