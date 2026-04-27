import * as vscode from 'vscode';
import { SchemeNotebookSerializer } from '../notebookSerializer';

// Mocking vscode module
jest.mock('vscode', () => jest.requireActual('../__mocks__/vscode'), { virtual: true });

describe('SchemeNotebookSerializer', () => {
    let serializer: SchemeNotebookSerializer;
    const token: vscode.CancellationToken = {
        isCancellationRequested: false,
        onCancellationRequested: jest.fn()
    };

    beforeEach(() => {
        serializer = new SchemeNotebookSerializer();
    });

    it('deserializes an empty file to a single empty code cell', async () => {
        const content = new Uint8Array();
        const data = await serializer.deserializeNotebook(content, token);

        expect(data.cells.length).toBe(1);
        expect(data.cells[0].kind).toBe(vscode.NotebookCellKind.Code);
        expect(data.cells[0].value).toBe('');
    });

    it('deserializes a file with only code', async () => {
        const content = new TextEncoder().encode('#lang racket\n(+ 1 2)\n');
        const data = await serializer.deserializeNotebook(content, token);

        expect(data.cells.length).toBe(1);
        expect(data.cells[0].kind).toBe(vscode.NotebookCellKind.Code);
        expect(data.cells[0].value).toBe('#lang racket\n(+ 1 2)');
    });

    it('deserializes a file with code and a markdown block', async () => {
        const text = `#lang racket\n(+ 1 2)\n\n#| markdown\nThis is markdown\n|#\n\n(+ 3 4)\n`;
        const content = new TextEncoder().encode(text);
        const data = await serializer.deserializeNotebook(content, token);

        expect(data.cells.length).toBe(3);
        
        expect(data.cells[0].kind).toBe(vscode.NotebookCellKind.Code);
        expect(data.cells[0].value).toBe('#lang racket\n(+ 1 2)');

        expect(data.cells[1].kind).toBe(vscode.NotebookCellKind.Markup);
        expect(data.cells[1].value).toBe('This is markdown');

        expect(data.cells[2].kind).toBe(vscode.NotebookCellKind.Code);
        expect(data.cells[2].value).toBe('(+ 3 4)');
    });

    it('deserializes a file starting with a markdown block', async () => {
        const text = `#| markdown\nStart\n|#\n(+ 1 2)\n`;
        const content = new TextEncoder().encode(text);
        const data = await serializer.deserializeNotebook(content, token);

        expect(data.cells.length).toBe(2);
        
        expect(data.cells[0].kind).toBe(vscode.NotebookCellKind.Markup);
        expect(data.cells[0].value).toBe('Start');

        expect(data.cells[1].kind).toBe(vscode.NotebookCellKind.Code);
        expect(data.cells[1].value).toBe('(+ 1 2)');
    });

    it('serializes notebook cells back to string', async () => {
        const cells = [
            new vscode.NotebookCellData(vscode.NotebookCellKind.Code, '#lang racket\n(+ 1 2)', 'racket'),
            new vscode.NotebookCellData(vscode.NotebookCellKind.Markup, 'Hello', 'markdown'),
            new vscode.NotebookCellData(vscode.NotebookCellKind.Code, '(+ 3 4)', 'racket')
        ];
        const data = new vscode.NotebookData(cells);

        const content = await serializer.serializeNotebook(data, token);
        const text = new TextDecoder().decode(content);

        expect(text).toBe('#lang racket\n(+ 1 2)\n\n#| markdown\nHello\n|#\n\n(+ 3 4)\n');
    });

    describe('Edge Cases', () => {
        it('handles unclosed markdown blocks', async () => {
            const text = `#lang racket\n#| markdown\nThis never ends`;
            const content = new TextEncoder().encode(text);
            const data = await serializer.deserializeNotebook(content, token);

            expect(data.cells.length).toBe(2);
            expect(data.cells[1].kind).toBe(vscode.NotebookCellKind.Markup);
            expect(data.cells[1].value).toBe('This never ends');
        });

        it('handles markdown header without newline', async () => {
            const text = `#| markdown Inline content |#`;
            const content = new TextEncoder().encode(text);
            const data = await serializer.deserializeNotebook(content, token);

            expect(data.cells.length).toBe(1);
            expect(data.cells[0].kind).toBe(vscode.NotebookCellKind.Markup);
            expect(data.cells[0].value).toBe('Inline content');
        });

        it('handles multiple consecutive markdown blocks', async () => {
            const text = `#| markdown 1 |#\n#| markdown 2 |#`;
            const content = new TextEncoder().encode(text);
            const data = await serializer.deserializeNotebook(content, token);

            expect(data.cells.length).toBe(2);
            expect(data.cells[0].value).toBe('1');
            expect(data.cells[1].value).toBe('2');
        });
    });
});
