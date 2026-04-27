import * as vscode from 'vscode';
import { SchemeNotebookController } from '../notebookController';

// Mocking vscode module
jest.mock('vscode', () => jest.requireActual('../__mocks__/vscode'), { virtual: true });

describe('SchemeNotebookController', () => {
    let controller: SchemeNotebookController;
    let mockClient: any;
    let mockExecution: any;
    let mockNotebookController: any;

    beforeEach(() => {
        mockClient = {
            sendNotification: jest.fn().mockResolvedValue(undefined)
        };
        mockExecution = {
            executionOrder: 1,
            start: jest.fn(),
            clearOutput: jest.fn(),
            appendOutput: jest.fn().mockResolvedValue(undefined),
            end: jest.fn()
        };

        // Mock createNotebookController to return a mock controller
        mockNotebookController = {
            supportedLanguages: [],
            supportsExecutionOrder: false,
            executeHandler: undefined,
            interruptHandler: undefined,
            createNotebookCellExecution: jest.fn().mockReturnValue(mockExecution),
            dispose: jest.fn()
        };
        (vscode.notebooks.createNotebookController as jest.Mock).mockReturnValue(mockNotebookController);

        controller = new SchemeNotebookController(() => mockClient);
    });

    afterEach(() => {
        jest.clearAllMocks();
    });

    describe('Execution Lifecycle', () => {
        it('should start execution and send notification to LSP', async () => {
            const cell = {
                document: { 
                    uri: { toString: () => 'cell-uri' }, 
                    getText: () => '(+ 1 2)', 
                    version: 1 
                },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;

            const executeHandler = mockNotebookController.executeHandler;
            await executeHandler([cell], {}, {});

            expect(mockExecution.start).toHaveBeenCalled();
            expect(mockExecution.clearOutput).toHaveBeenCalled();
            expect(mockClient.sendNotification).toHaveBeenCalledWith('scheme/notebook/evalCell', expect.objectContaining({
                uri: 'cell-uri',
                code: '(+ 1 2)'
            }));
        });

        it('should end execution immediately if LSP client is missing', async () => {
            controller = new SchemeNotebookController(() => undefined);
            const cell = {
                document: { uri: { toString: () => 'cell-uri' } },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;

            const executeHandler = mockNotebookController.executeHandler;
            await executeHandler([cell], {}, {});

            expect(mockExecution.end).toHaveBeenCalledWith(false, expect.any(Number));
            expect(vscode.window.showErrorMessage).toHaveBeenCalledWith(expect.stringContaining("LSP Client not running"));
        });
    });

    describe('handleOutputStream', () => {
        it('should append stdout to matching execution', async () => {
            const cell = {
                document: { uri: { toString: () => 'cell-uri' }, getText: () => 'code', version: 1 },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;
            
            await mockNotebookController.executeHandler([cell], {}, {});

            await controller.handleOutputStream({
                executionId: 1,
                payload: { type: 'stdout', data: 'hello' }
            });

            expect(mockExecution.appendOutput).toHaveBeenCalled();
            const output = mockExecution.appendOutput.mock.calls[0][0];
            expect(output.items[0].data.toString()).toContain('hello');
        });

        it('should handle rich output (png)', async () => {
            const cell = {
                document: { uri: { toString: () => 'cell-uri' }, getText: () => 'code', version: 1 },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;
            await mockNotebookController.executeHandler([cell], {}, {});

            await controller.handleOutputStream({
                executionId: 1,
                payload: { type: 'rich', mime: 'image/png', data: Buffer.from('fake-png').toString('base64') }
            });

            expect(mockExecution.appendOutput).toHaveBeenCalled();
            const output = mockExecution.appendOutput.mock.calls[0][0];
            expect(output.items[0].mime).toBe('image/png');
        });

        it('should handle evaluation errors', async () => {
            const cell = {
                document: { uri: { toString: () => 'cell-uri' }, getText: () => 'code', version: 1 },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;
            await mockNotebookController.executeHandler([cell], {}, {});

            await controller.handleOutputStream({
                executionId: 1,
                payload: { type: 'error', data: 'bad things happened' }
            });

            expect(mockExecution.appendOutput).toHaveBeenCalled();
            const output = mockExecution.appendOutput.mock.calls[0][0];
            expect(output.items[0].mime).toBe('application/vnd.code.notebook.error');
        });
    });

    describe('handleEvalFinished', () => {
        it('should end execution and remove from active map', async () => {
            const cell = {
                document: { uri: { toString: () => 'cell-uri' }, getText: () => 'code', version: 1 },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;
            await mockNotebookController.executeHandler([cell], {}, {});

            controller.handleEvalFinished({
                executionId: 1,
                success: true
            });

            expect(mockExecution.end).toHaveBeenCalledWith(true, expect.any(Number));
            
            // Verify it was removed from map by trying to send output to it
            mockExecution.appendOutput.mockClear();
            await controller.handleOutputStream({
                executionId: 1,
                payload: { type: 'stdout', data: 'post-finish' }
            });
            expect(mockExecution.appendOutput).not.toHaveBeenCalled();
        });
    });

    describe('_interrupt', () => {
        it('should send cancelEval notification for all active cells', async () => {
            const cell = {
                document: { uri: { toString: () => 'cell-uri' }, getText: () => 'code', version: 1 },
                notebook: { uri: { toString: () => 'nb-uri' } }
            } as any;
            const mockNotebook = {
                getCells: () => [cell]
            } as any;

            await mockNotebookController.executeHandler([cell], {}, {});

            const interruptHandler = mockNotebookController.interruptHandler;
            interruptHandler(mockNotebook);

            expect(mockClient.sendNotification).toHaveBeenCalledWith('scheme/notebook/cancelEval', expect.objectContaining({
                uri: 'cell-uri',
                executionId: 1
            }));
        });
    });
});
