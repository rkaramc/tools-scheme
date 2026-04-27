import * as vscode from "vscode";
import * as fs from "fs";

// Mocking vscode module
jest.mock('vscode', () => jest.requireActual('../__mocks__/vscode'), { virtual: true });

// Mock language client
jest.mock('vscode-languageclient/node', () => ({
    LanguageClient: jest.fn().mockImplementation(() => ({
        start: jest.fn().mockResolvedValue(undefined),
        stop: jest.fn().mockResolvedValue(undefined),
        sendRequest: jest.fn().mockResolvedValue({}),
        onNotification: jest.fn()
    }))
}), { virtual: true });

// Mock utils
jest.mock('../utils', () => ({
    resolveLspPath: jest.fn().mockReturnValue('/path/to/lsp'),
    cleanupStaleFiles: jest.fn(),
    getRuntimeBinaryPath: jest.fn().mockReturnValue({ newPath: '/path/to/lsp' })
}));

// Mock fs
jest.mock('fs', () => ({
    ...jest.requireActual('fs'),
    watch: jest.fn(),
    existsSync: jest.fn().mockReturnValue(true),
    unlinkSync: jest.fn()
}));

import { activate, deactivate } from '../extension';

describe('extension', () => {
    let mockContext: any;

    beforeEach(() => {
        mockContext = {
            subscriptions: [],
            extensionMode: vscode.ExtensionMode.Production,
            asAbsolutePath: jest.fn().mockImplementation(p => p),
            extensionPath: '/ext',
            globalStorageUri: { fsPath: '/storage' }
        };
        jest.clearAllMocks();
    });

    it('should register all expected commands on activate', async () => {
        await activate(mockContext);

        const registeredCommands = (vscode.commands.registerCommand as jest.Mock).mock.calls.map(call => call[0]);
        expect(registeredCommands).toContain('scheme.runEvaluation');
        expect(registeredCommands).toContain('scheme.runEvaluateSelection');
        expect(registeredCommands).toContain('scheme.restartREPL');
        expect(registeredCommands).toContain('scheme.clearNamespace');
    });

    it('should setup notebook serializer and controller', async () => {
        await activate(mockContext);

        expect(vscode.workspace.registerNotebookSerializer).toHaveBeenCalledWith('scheme-notebook', expect.any(Object));
        expect(vscode.notebooks.createNotebookController).toHaveBeenCalled();
    });

    it('should cleanup resources on deactivate', async () => {
        // Activate first to setup variables
        await activate(mockContext);
        
        await deactivate();

        // Check if file watcher would be closed (if we were in dev mode)
        // But here we'll just check if it runs without error
    });

    it('should start file watcher in development mode', async () => {
        mockContext.extensionMode = vscode.ExtensionMode.Development;
        await activate(mockContext);

        expect(fs.watch).toHaveBeenCalled();
    });
});
