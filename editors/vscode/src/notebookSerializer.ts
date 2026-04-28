import * as vscode from 'vscode';
import { TextDecoder, TextEncoder } from 'util';

export class SchemeNotebookSerializer implements vscode.NotebookSerializer {
    private readonly decoder = new TextDecoder();
    private readonly encoder = new TextEncoder();

    public async deserializeNotebook(
        content: Uint8Array,
        _token: vscode.CancellationToken
    ): Promise<vscode.NotebookData> {
        const str = this.decoder.decode(content);
        const cells: vscode.NotebookCellData[] = [];

        // Determine the language ID for code cells based on the file extension
        // Since we don't have direct access to the URI here easily, 
        // we can default to 'racket-notebook-cell' and let the notebook type handle it.
        // Actually, the NotebookSerializer is registered for 'scheme-notebook'.
        // We'll use 'racket-notebook-cell' as the default for now.
        const langId = 'racket-notebook-cell';

        let currentIndex = 0;
        const markdownStartRegex = /#\|\s*markdown\s*\n?/g;

        while (currentIndex < str.length) {
            markdownStartRegex.lastIndex = currentIndex;
            const match = markdownStartRegex.exec(str);

            if (match) {
                // Code before the markdown block
                const codePart = str.substring(currentIndex, match.index);
                const cleanCodePart = codePart.replace(/^\s*\n/, '').trimEnd();
                
                if (cleanCodePart.length > 0) {
                    const blocks = cleanCodePart.split(/(?:\r?\n[ \t]*){2,}/);
                    for (const block of blocks) {
                        if (block.trim().length > 0) {
                            cells.push(new vscode.NotebookCellData(vscode.NotebookCellKind.Code, block.trimEnd(), langId));
                        }
                    }
                }

                const mdContentStart = match.index + match[0].length;
                const endBlockIndex = str.indexOf('|#', mdContentStart);

                if (endBlockIndex !== -1) {
                    const mdContent = str.substring(mdContentStart, endBlockIndex);
                    cells.push(new vscode.NotebookCellData(vscode.NotebookCellKind.Markup, mdContent.trim(), 'markdown'));
                    currentIndex = endBlockIndex + 2;
                } else {
                    // Unclosed markdown block
                    const mdContent = str.substring(mdContentStart);
                    cells.push(new vscode.NotebookCellData(vscode.NotebookCellKind.Markup, mdContent.trim(), 'markdown'));
                    currentIndex = str.length;
                }
            } else {
                // Remaining code
                const codePart = str.substring(currentIndex);
                const cleanCodePart = codePart.replace(/^\s*\n/, '').trimEnd();
                if (cleanCodePart.length > 0) {
                    const blocks = cleanCodePart.split(/(?:\r?\n[ \t]*){2,}/);
                    for (const block of blocks) {
                        if (block.trim().length > 0) {
                            cells.push(new vscode.NotebookCellData(vscode.NotebookCellKind.Code, block.trimEnd(), langId));
                        }
                    }
                } else if (cells.length === 0) {
                    cells.push(new vscode.NotebookCellData(vscode.NotebookCellKind.Code, '', langId));
                }
                currentIndex = str.length;
            }
        }

        if (cells.length === 0) {
            cells.push(new vscode.NotebookCellData(vscode.NotebookCellKind.Code, '', langId));
        }

        return new vscode.NotebookData(cells);
    }

    public async serializeNotebook(
        data: vscode.NotebookData,
        _token: vscode.CancellationToken
    ): Promise<Uint8Array> {
        let contents = '';

        for (const cell of data.cells) {
            if (cell.kind === vscode.NotebookCellKind.Markup) {
                contents += `#| markdown\n${cell.value}\n|#\n\n`;
            } else {
                contents += `${cell.value}\n\n`;
            }
        }

        return this.encoder.encode(contents.trimEnd() + '\n');
    }
}
