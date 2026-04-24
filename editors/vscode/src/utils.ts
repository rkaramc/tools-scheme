import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";
import * as crypto from "crypto";

const TEMP_DIR_NAME = "vscode-scheme-toolbox-lsp";

/**
 * Returns the path to the dedicated temporary directory for LSP binaries.
 * @param create If true, creates the directory if it doesn't exist.
 */
export function getTempDir(create = true): string {
  const tempDir = path.join(os.tmpdir(), TEMP_DIR_NAME);
  if (create) {
    ensureTempDirExists(tempDir);
  }
  return tempDir;
}

/**
 * Creates the directory for LSP binaries.
 */
export function ensureTempDirExists(tempDir: string): void {
  if (!fs.existsSync(tempDir)) {
    fs.mkdirSync(tempDir, { recursive: true });
  }
}

/**
 * Attempts to clean up stale temporary LSP binaries from previous sessions.
 * @param outputChannel The output channel to log cleanup progress to.
 * @param tempDir The directory to clean up. If omitted, uses the default temporary directory.
 */
export function cleanupStaleFiles(outputChannel: vscode.OutputChannel, tempDir?: string) {
  tempDir = tempDir || getTempDir();
  if (!fs.existsSync(tempDir)) {
    return;
  }

  const files = fs.readdirSync(tempDir);
  for (const file of files) {
    // Broaden the pattern to ensure we catch the test files, 
    // even if it picks up some system files (which we will ignore)
    if (
        file.startsWith("scheme-toolbox-lsp-") ||
        file.startsWith("eval-shim-")
    ) {
      const filePath = path.join(tempDir, file);
      try {
        fs.unlinkSync(filePath);
        outputChannel.appendLine(`Cleaned up stale temporary file: ${file}`);
      } catch (err) {
        // Ignore files that are in use (common in system temp)
      }
    }
  }
}

/**
 * Searches for a binary in the system PATH.
 */
export function findInPath(binaryName: string): string | undefined {
  const paths = (process.env.PATH || process.env.Path || "").split(
    path.delimiter,
  );
  for (const p of paths) {
    const fullPath = path.join(p, binaryName);
    if (fs.existsSync(fullPath)) {
      return fullPath;
    }
  }
  return undefined;
}

/**
 * Resolves the path to the LSP binary, checking settings, environment, PATH, cargo home, and development fallback.
 */
export function resolveLspPath(context: vscode.ExtensionContext): string | undefined {
  const binName =
    process.platform === "win32"
      ? "scheme-toolbox-lsp.exe"
      : "scheme-toolbox-lsp";

  // 0. VS Code extension settings
  const config = vscode.workspace.getConfiguration("scheme");
  const customLspPath = config.get<string>("lspPath");
  let serverPath = customLspPath;

  // 1. Development override
  if (
    !serverPath &&
    context.extensionMode === vscode.ExtensionMode.Development
  ) {
    const devPath = context.asAbsolutePath(
      path.join("..", "..", "target", "debug", binName),
    );
    if (fs.existsSync(devPath)) {
      serverPath = devPath;
    }
  }

  // 2. Environment Variable override
  const envLspDir = process.env.TOOLS_SCHEME_LSP_PATH;
  if (!serverPath && envLspDir) {
    const envPath = path.join(envLspDir, binName);
    if (fs.existsSync(envPath)) {
      serverPath = envPath;
    }
  }

  // 3. System PATH
  if (!serverPath) {
    serverPath = findInPath(binName);
  }

  // 4. Common installation paths (e.g., Cargo home)
  if (!serverPath) {
    const homeDir = os.homedir();
    const cargoBinPath = path.join(homeDir, ".cargo", "bin", binName);
    if (fs.existsSync(cargoBinPath)) {
      serverPath = cargoBinPath;
    }
  }

  return serverPath;
}

/**
 * Prepares the binary for execution. On Windows in Development mode, this involves
 * copying the binary to a temporary location to avoid locking.
 */
export function getRuntimeBinaryPath(
  context: vscode.ExtensionContext,
  originalPath: string,
  outputChannel: vscode.OutputChannel,
  currentTempPath?: string,
): { newPath: string; updatedTempPath?: string } {
  const isDevelopment =
    context.extensionMode === vscode.ExtensionMode.Development;
  if (!isDevelopment || process.platform !== "win32") {
    return { newPath: originalPath };
  }

  try {
    const randomSuffix = crypto.randomBytes(8).toString("hex");
    const tempName = `scheme-toolbox-lsp-${randomSuffix}.exe`;
    const newTempPath = path.join(getTempDir(), tempName);
    fs.copyFileSync(originalPath, newTempPath);

    // Cleanup old temp file if it exists
    if (currentTempPath && fs.existsSync(currentTempPath)) {
      try {
        fs.unlinkSync(currentTempPath);
      } catch (e) {
        // unlink is trying to remove server binary created earlier by this instance
        // on win32, unlink fails if this instance's server process is running
      }
    }

    outputChannel.appendLine(
      `Copied LSP binary to temporary location: ${newTempPath}`,
    );
    return { newPath: newTempPath, updatedTempPath: newTempPath };
  } catch (err) {
    outputChannel.appendLine(
      `Failed to copy LSP binary to temporary location: ${err}`,
    );
    return { newPath: originalPath };
  }
}
