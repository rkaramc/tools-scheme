import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

// Mocking vscode module
jest.mock('vscode', () => jest.requireActual('../__mocks__/vscode'), { virtual: true });

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

const mockOutputChannel = {
    appendLine: jest.fn()
} as any;

describe("utils", () => {
    afterEach(() => {
        jest.restoreAllMocks();
        jest.clearAllMocks();
        delete process.env.TOOLS_SCHEME_LSP_PATH;
    });

    describe("getTempDir", () => {
        it("should return the path from TOOLS_SCHEME_TMP_DIR if set", () => {
            const customDir = "/custom/tmp/dir";
            process.env.TOOLS_SCHEME_TMP_DIR = customDir;
            (fs.existsSync as jest.Mock).mockReturnValue(true);
            
            expect(utils.getTempDir(false)).toBe(customDir);
        });

        it("should fallback to system temp dir if TOOLS_SCHEME_TMP_DIR is not set", () => {
            delete process.env.TOOLS_SCHEME_TMP_DIR;
            const systemTemp = os.tmpdir();
            
            const result = utils.getTempDir(false);
            expect(result).toContain(systemTemp);
            expect(result).toContain("vscode-scheme-toolbox-lsp");
        });
    });

    describe("cleanupStaleFiles", () => {
        let tempDir: string;

        beforeEach(() => {
            tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "vscode-scheme-toolbox-lsp-"));
            jest.spyOn(utils, 'getTempDir').mockReturnValue(tempDir);
            (fs.existsSync as jest.Mock).mockImplementation(jest.requireActual('fs').existsSync);
        });

        afterEach(() => {
            fs.rmSync(tempDir, { recursive: true, force: true });
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
        let context: any;
        const binName = process.platform === "win32" ? "scheme-toolbox-lsp.exe" : "scheme-toolbox-lsp";

        beforeEach(() => {
            context = {
                extensionMode: vscode.ExtensionMode.Production,
                asAbsolutePath: jest.fn().mockImplementation(p => path.join("/extension", p))
            };
            (vscode.workspace.getConfiguration as jest.Mock).mockReturnValue({
                get: jest.fn().mockReturnValue(undefined)
            });
            (fs.existsSync as jest.Mock).mockReturnValue(false);
        });

        it("should return custom path from settings if provided", () => {
            (vscode.workspace.getConfiguration as jest.Mock).mockReturnValue({
                get: jest.fn().mockReturnValue("/custom/lsp")
            });
            expect(resolveLspPath(context)).toBe("/custom/lsp");
        });

        it("should fallback to dev path in development mode", () => {
            context.extensionMode = vscode.ExtensionMode.Development;
            const devPath = path.join("/extension", "..", "..", "target", "debug", binName);
            (fs.existsSync as jest.Mock).mockImplementation(p => p === devPath);

            expect(resolveLspPath(context)).toBe(devPath);
        });

        it("should fallback to environment variable if provided", () => {
            process.env.TOOLS_SCHEME_LSP_PATH = "/env/bin";
            const envPath = path.join("/env/bin", binName);
            (fs.existsSync as jest.Mock).mockImplementation(p => p === envPath);

            expect(resolveLspPath(context)).toBe(envPath);
        });

        it("should fallback to system PATH", () => {
            const originalPath = process.env.PATH;
            process.env.PATH = "/usr/bin";
            const binPath = path.join("/usr/bin", binName);
            (fs.existsSync as jest.Mock).mockImplementation(p => p === binPath);

            expect(resolveLspPath(context)).toBe(binPath);
            process.env.PATH = originalPath;
        });

        it("should fallback to Cargo home", () => {
            const cargoPath = path.join(os.homedir(), ".cargo", "bin", binName);
            (fs.existsSync as jest.Mock).mockImplementation(p => p === cargoPath);

            expect(resolveLspPath(context)).toBe(cargoPath);
        });
    });

    describe("getRuntimeBinaryPath", () => {
        let context: any;

        beforeEach(() => {
            context = { extensionMode: vscode.ExtensionMode.Production };
        });

        it("should return original path in production", () => {
            const result = getRuntimeBinaryPath(context, "/orig", mockOutputChannel);
            expect(result.newPath).toBe("/orig");
        });

        if (process.platform === "win32") {
            it("should copy binary to temp in development on Windows", () => {
                context.extensionMode = vscode.ExtensionMode.Development;
                const testTempDir = path.join(os.tmpdir(), "test-lsp-bin");
                jest.spyOn(utils, 'getTempDir').mockReturnValue(testTempDir);
                (fs.copyFileSync as jest.Mock).mockImplementation(() => { /* no-op */ });

                const result = getRuntimeBinaryPath(context, "C:\\bin\\lsp.exe", mockOutputChannel);
                
                expect(result.newPath).toContain("vscode-scheme-toolbox-lsp");
                expect(result.newPath).toContain("scheme-toolbox-lsp-");
                expect(fs.copyFileSync).toHaveBeenCalled();
            });

            it("should cleanup previous temp path if provided", () => {
                context.extensionMode = vscode.ExtensionMode.Development;
                const oldTemp = "C:\\tmp\\old.exe";
                (fs.existsSync as jest.Mock).mockImplementation(p => p === oldTemp);
                (fs.unlinkSync as jest.Mock).mockImplementation(() => { /* no-op */ });

                getRuntimeBinaryPath(context, "C:\\bin\\lsp.exe", mockOutputChannel, oldTemp);
                
                expect(fs.unlinkSync).toHaveBeenCalledWith(oldTemp);
            });

            it("should handle copy failure gracefully", () => {
                context.extensionMode = vscode.ExtensionMode.Development;
                (fs.copyFileSync as jest.Mock).mockImplementation(() => { throw new Error("Copy failed"); });

                const result = getRuntimeBinaryPath(context, "C:\\bin\\lsp.exe", mockOutputChannel);
                
                expect(result.newPath).toBe("C:\\bin\\lsp.exe");
                expect(mockOutputChannel.appendLine).toHaveBeenCalledWith(expect.stringContaining("Failed to copy"));
            });
        }
    });
});
