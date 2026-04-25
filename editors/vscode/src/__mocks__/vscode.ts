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

