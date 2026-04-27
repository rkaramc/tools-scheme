import * as vscode from "vscode";

export const workspace = {
    getConfiguration: jest.fn().mockReturnValue({
        get: jest.fn(),
        affectsConfiguration: jest.fn().mockReturnValue(false)
    }),
    registerNotebookSerializer: jest.fn().mockReturnValue({ dispose: jest.fn() }),
    onDidOpenTextDocument: jest.fn().mockReturnValue({ dispose: jest.fn() }),
    onDidChangeConfiguration: jest.fn().mockReturnValue({ dispose: jest.fn() }),
    createFileSystemWatcher: jest.fn().mockReturnValue({ dispose: jest.fn() }),
    textDocuments: []
};

export const window = {
    createOutputChannel: jest.fn().mockReturnValue({
        appendLine: jest.fn(),
        dispose: jest.fn(),
        append: jest.fn(),
        clear: jest.fn(),
        show: jest.fn(),
        hide: jest.fn(),
        replace: jest.fn()
    }),
    showErrorMessage: jest.fn(),
    showInformationMessage: jest.fn(),
    activeTextEditor: undefined as any
};

export const commands = {
    registerCommand: jest.fn().mockReturnValue({ dispose: jest.fn() }),
    executeCommand: jest.fn()
};

export const notebooks = {
    createNotebookController: jest.fn().mockReturnValue({
        dispose: jest.fn(),
        supportedLanguages: [],
        supportsExecutionOrder: false,
        executeHandler: undefined,
        interruptHandler: undefined,
        createNotebookCellExecution: jest.fn()
    })
};

export const languages = {
    setTextDocumentLanguage: jest.fn(),
    getDiagnostics: jest.fn().mockReturnValue([])
};

export const extensions = {
    getExtension: jest.fn()
};

export const Uri = {
    parse: jest.fn().mockImplementation((val) => ({ toString: () => val, fsPath: val })),
    file: jest.fn().mockImplementation((val) => ({ toString: () => `file://${val}`, fsPath: val }))
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

export class Range {
    start: Position;
    end: Position;
    constructor(startLine: number, startChar: number, endLine: number, endChar: number) {
        this.start = new Position(startLine, startChar);
        this.end = new Position(endLine, endChar);
    }
}

export class Position {
    line: number;
    character: number;
    constructor(line: number, character: number) {
        this.line = line;
        this.character = character;
    }
}

export class Location {
    uri: any;
    range: Range;
    constructor(uri: any, range: Range) {
        this.uri = uri;
        this.range = range;
    }
}

export enum DiagnosticSeverity {
    Error = 0,
    Warning = 1,
    Information = 2,
    Hint = 3
}
