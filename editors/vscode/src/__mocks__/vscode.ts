export const workspace = {
    getConfiguration: jest.fn().mockReturnValue({
        get: jest.fn()
    })
};
export const window = {
    createOutputChannel: jest.fn().mockReturnValue({
        appendLine: jest.fn()
    }),
    showErrorMessage: jest.fn(),
    showInformationMessage: jest.fn()
};
export const commands = {
    registerCommand: jest.fn()
};
export const notebooks = {
    createNotebookController: jest.fn()
};
export const Uri = {
    parse: jest.fn()
};
export enum ExtensionMode {
    Production = 1,
    Development = 2,
    Test = 3
}

export enum NotebookCellKind {
    Markup = 1,
    Code = 2
}

export class NotebookCellData {
    kind: NotebookCellKind;
    value: string;
    languageId: string;
    constructor(kind: NotebookCellKind, value: string, languageId: string) {
        this.kind = kind;
        this.value = value;
        this.languageId = languageId;
    }
}

export class NotebookCellOutputItem {
    static stdout(data: string): NotebookCellOutputItem {
        return new NotebookCellOutputItem(Buffer.from(data), 'application/vnd.code.notebook.stdout');
    }
    static stderr(data: string): NotebookCellOutputItem {
        return new NotebookCellOutputItem(Buffer.from(data), 'application/vnd.code.notebook.stderr');
    }
    static text(data: string, mime?: string): NotebookCellOutputItem {
        return new NotebookCellOutputItem(Buffer.from(data), mime || 'text/plain');
    }
    static error(data: any): NotebookCellOutputItem {
        return new NotebookCellOutputItem(Buffer.from(JSON.stringify(data)), 'application/vnd.code.notebook.error');
    }

    data: Uint8Array;
    mime: string;
    constructor(data: Uint8Array, mime: string) {
        this.data = data;
        this.mime = mime;
    }
}

export class NotebookCellOutput {
    items: NotebookCellOutputItem[];
    metadata?: { [key: string]: any };
    constructor(items: NotebookCellOutputItem[], metadata?: { [key: string]: any }) {
        this.items = items;
        this.metadata = metadata;
    }
}

export class NotebookData {
    cells: NotebookCellData[];
    constructor(cells: NotebookCellData[]) {
        this.cells = cells;
    }
}

export interface CancellationToken {
    isCancellationRequested: boolean;
    onCancellationRequested: any;
}

