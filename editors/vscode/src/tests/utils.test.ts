import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

// Mocking vscode.OutputChannel
const mockOutputChannel = {
    name: "Mock",
    append: (_value: string) => { return; },
    appendLine: (value: string) => { console.log(value); },
    replace: (_value: string) => { return; },
    clear: () => { return; },
    show: (_column?: any, _preserveFocus?: any) => { return; },
    hide: () => { return; },
    dispose: () => { return; }
} as any;

import { cleanupStaleFiles, resolveLspPath, findInPath, getRuntimeBinaryPath } from "../utils";
import * as utils from "../utils";

// Mock fs to control existsSync behavior
jest.mock('fs', () => {
    const realFs = jest.requireActual('fs');
    return {
        ...realFs,
        existsSync: jest.fn(realFs.existsSync),
        copyFileSync: jest.fn(realFs.copyFileSync),
        unlinkSync: jest.fn(realFs.unlinkSync),
        mkdirSync: jest.fn(realFs.mkdirSync),
        readdirSync: jest.fn(realFs.readdirSync),
        writeFileSync: jest.fn(realFs.writeFileSync),
        mkdtempSync: jest.fn(realFs.mkdtempSync),
        rmSync: jest.fn(realFs.rmSync),
    };
});

describe("utils", () => {
    describe("cleanupStaleFiles", () => {
        let tempDir: string;

        beforeEach(() => {
            // Setup a real temp directory for testing
            tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "vscode-scheme-toolbox-lsp-"));
            
            // Spy on getTempDir to return our test tempDir
            // Spy on the exported object itself
            jest.spyOn(utils, 'getTempDir').mockReturnValue(tempDir);
            (fs.existsSync as jest.Mock).mockImplementation(jest.requireActual('fs').existsSync);
        });

        afterEach(() => {
            // Clean up the test directory
            fs.rmSync(tempDir, { recursive: true, force: true });
            jest.restoreAllMocks();
        });

        it("should clean up files matching the pattern", () => {
            const file1 = path.join(tempDir, "scheme-toolbox-lsp-123.exe");
            const file2 = path.join(tempDir, "eval-shim-456.rkt");
            const keepFile = path.join(tempDir, "keep-me.txt");

            fs.writeFileSync(file1, "content");
            fs.writeFileSync(file2, "content");
            fs.writeFileSync(keepFile, "content");

            cleanupStaleFiles(mockOutputChannel, tempDir);

            expect(fs.existsSync(file1)).toBe(false);
            expect(fs.existsSync(file2)).toBe(false);
            expect(fs.existsSync(keepFile)).toBe(true);
        });
    });

    describe("resolveLspPath", () => {
        it("should return undefined if no path is found", () => {
            const context = {
                extensionMode: vscode.ExtensionMode.Production,
                asAbsolutePath: () => "/fake/path"
            } as any;
            
            // Mock fs.existsSync to always return false
            (fs.existsSync as jest.Mock).mockReturnValue(false);
            
            expect(resolveLspPath(context)).toBeUndefined();
        });
    });

    describe("findInPath", () => {
        it("should return the path if binary is found", () => {
            const originalPath = process.env.PATH;
            const binDir = path.normalize("/usr/bin");
            const binPath = path.join(binDir, "test-bin");
            
            process.env.PATH = `/bin${path.delimiter}${binDir}`;
            (fs.existsSync as jest.Mock).mockImplementation((p: string) => p === binPath);

            const result = findInPath("test-bin");
            expect(result).toBe(binPath);

            process.env.PATH = originalPath;
        });
    });

    describe("getRuntimeBinaryPath", () => {
        it("should return original path in production", () => {
            const context = { extensionMode: vscode.ExtensionMode.Production } as any;
            const result = getRuntimeBinaryPath(context, "/orig", mockOutputChannel);
            expect(result.newPath).toBe("/orig");
        });
    });
});
