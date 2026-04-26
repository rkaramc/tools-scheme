import * as assert from "assert";
import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

suite("Extension Integration Test Suite", () => {
  vscode.window.showInformationMessage("Start all tests.");

  test("Notebook Cell Isolation & Diagnostic Test", async function (this: any) {
    this.timeout(120000); // 2 minutes for full end-to-end robustness
    // Create a temporary notebook file in the OS temp directory
    const testNbPath = path.join(os.tmpdir(), "test_temp.rktnb");

    // Initial content for the notebook
    // We'll use displayln to be absolutely sure we see something in stdout
    const content = '(define x 10)\n(displayln (format "VAL:~a" x))\n\n#| markdown\nHello\n|#\n\n(+ x 5)';
    fs.writeFileSync(testNbPath, content);

    try {
      // 1. Ensure extension is activated
      const ext = vscode.extensions.getExtension("rkaramc.tools-scheme");
      assert.ok(ext, "Extension not found!");
      await ext.activate();

      const uri = vscode.Uri.file(testNbPath);
      const doc = await vscode.workspace.openNotebookDocument(uri);
      await vscode.window.showNotebookDocument(doc);

      // Give LSP a generous moment to connect and initialize (especially on first run)
      await new Promise((resolve) => setTimeout(resolve, 10000));

      // Verify cell count (3 cells based on our serializer logic: code, md, code)
      assert.strictEqual(doc.cellCount, 3);
      assert.strictEqual(doc.cellAt(0).kind, vscode.NotebookCellKind.Code);
      assert.strictEqual(doc.cellAt(1).kind, vscode.NotebookCellKind.Markup);
      assert.strictEqual(doc.cellAt(2).kind, vscode.NotebookCellKind.Code);

      // Check current language ID (should be 'racket-notebook-cell' now)
      assert.strictEqual(
        doc.cellAt(0).document.languageId,
        "racket-notebook-cell",
      );

      // Verify that we can execute the cells and check shared state
      const firstCell = doc.cellAt(0);
      const lastCell = doc.cellAt(2);

      await vscode.commands.executeCommand("notebook.execute", doc.uri);

      // 1. Poll for first cell completion OR correct output
      let firstSuccess = false;
      for (let i = 0; i < 30; i++) {
        const hasResult = firstCell.outputs.some(o => 
          o.items.some(item => new TextDecoder().decode(item.data).includes("VAL:10"))
        );
        if (firstCell.executionSummary?.success === true || hasResult) {
          firstSuccess = true;
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      assert.strictEqual(firstSuccess, true, "First cell execution failed or timed out");

      // 2. Poll for last cell completion OR correct output (id=2)
      let lastSuccess = false;
      for (let i = 0; i < 30; i++) {
        const summary = lastCell.executionSummary;
        const hasResult = lastCell.outputs.some(o => 
          o.items.some(item => new TextDecoder().decode(item.data).includes("15"))
        );

        if (summary?.success === true || hasResult) {
          lastSuccess = true;
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      assert.strictEqual(lastSuccess, true, "Last cell (shared state) execution failed or timed out");

      // 3. Verify diagnostics are re-enabled and downgraded to Warning for notebooks
      const edit = new vscode.WorkspaceEdit();
      const cellData = new vscode.NotebookCellData(
        vscode.NotebookCellKind.Code,
        '(define-values (y y) (values 1 2))',
        "racket-notebook-cell",
      );
      const notebookEdit = vscode.NotebookEdit.insertCells(doc.cellCount, [cellData]);
      edit.set(doc.uri, [notebookEdit]);
      await vscode.workspace.applyEdit(edit);
      
      const errorCell = doc.cellAt(doc.cellCount - 1);
      await vscode.commands.executeCommand("notebook.execute", doc.uri);

      // Wait for diagnostic to appear (polling)
      let foundDiag = false;
      for (let i = 0; i < 30; i++) {
        const diagnostics = vscode.languages.getDiagnostics(errorCell.document.uri);
        if (diagnostics.length > 0) {
          assert.strictEqual(diagnostics[0].severity, vscode.DiagnosticSeverity.Warning, "Duplicate identifier should be a WARNING in notebook");
          foundDiag = true;
          break;
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(foundDiag, true, "Should have found diagnostics for duplicate identifier in notebook cell");

      // Pause to allow visual inspection if requested
      if (process.env.VSCODE_TEST_PAUSE) {
        const pauseTime = parseInt(process.env.VSCODE_TEST_PAUSE) || 5000;
        await new Promise(resolve => setTimeout(resolve, pauseTime));
      }
    } finally {
      if (fs.existsSync(testNbPath)) {
        fs.unlinkSync(testNbPath);
      }
    }
  });
});
