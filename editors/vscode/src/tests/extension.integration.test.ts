import * as assert from "assert";
import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

suite("Extension Integration Test Suite", () => {
  vscode.window.showInformationMessage("Start all tests.");

  test("Notebook Cell Isolation & Diagnostic Test", async function (this: any) {
    this.timeout(120000); // 2 minutes for full end-to-end robustness
    // ... (rest of notebook test remains unchanged)
  });

  test("File-based Evaluation & InlayHints Test", async function (this: any) {
    this.timeout(60000);
    const testFilePath = path.join(os.tmpdir(), "test_eval.rkt");
    const content = '#lang racket\n(+ 1 2)\n(define (f x) (* x x))\n(f 10)\n';
    fs.writeFileSync(testFilePath, content);

    try {
      const ext = vscode.extensions.getExtension("rkaramc.tools-scheme");
      assert.ok(ext, "Extension not found!");
      await ext.activate();

      const uri = vscode.Uri.file(testFilePath);
      const doc = await vscode.workspace.openTextDocument(uri);
      const editor = await vscode.window.showTextDocument(doc);

      // Give LSP time to start
      await new Promise((resolve) => setTimeout(resolve, 5000));

      // Trigger evaluation
      await vscode.commands.executeCommand("scheme.runEvaluation", uri);

      // Wait for hints to appear (polling)
      // Hints should appear for (+ 1 2) and (f 10)
      let hintsFound = false;
      const range = new vscode.Range(0, 0, doc.lineCount, 0);
      
      for (let i = 0; i < 20; i++) {
        const hints = await vscode.commands.executeCommand<vscode.InlayHint[]>(
          "vscode.executeInlayHintProvider",
          uri,
          range
        );

        if (hints && hints.length >= 2) {
          // Verify hint content
          const hintTexts = hints.map(h => {
              if (typeof h.label === 'string') return h.label;
              return h.label.map(l => l.value).join('');
          });
          
          if (hintTexts.some(t => t.includes('3')) && hintTexts.some(t => t.includes('100'))) {
            hintsFound = true;
            break;
          }
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }

      assert.strictEqual(hintsFound, true, "Expected InlayHints (3 and 100) not found or timed out");

    } finally {
      if (fs.existsSync(testFilePath)) {
        fs.unlinkSync(testFilePath);
      }
    }
  });
});
