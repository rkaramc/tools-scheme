import * as assert from "assert";
import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

suite("Extension Integration Test Suite", () => {
  vscode.window.showInformationMessage("Start all tests.");

  test("Notebook Cell Isolation & Diagnostic Test", async function (this: any) {
    this.timeout(120000); 
    const testNbPath = path.join(os.tmpdir(), `test_temp_${Date.now()}.rktnb`);
    const content = '(define x 10)\n(displayln (format "VAL:~a" x))\n\n#| markdown\nHello\n|#\n\n(+ x 5)';
    fs.writeFileSync(testNbPath, content);

    try {
      const ext = vscode.extensions.getExtension("rkaramc.tools-scheme");
      assert.ok(ext, "Extension not found!");
      await ext.activate();

      const uri = vscode.Uri.file(testNbPath);
      const doc = await vscode.workspace.openNotebookDocument(uri);
      await vscode.window.showNotebookDocument(doc);

      await new Promise((resolve) => setTimeout(resolve, 10000));

      assert.strictEqual(doc.cellCount, 3);
      const firstCell = doc.cellAt(0);
      const lastCell = doc.cellAt(2);

      await vscode.commands.executeCommand("notebook.execute", doc.uri);

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

      let lastSuccess = false;
      for (let i = 0; i < 30; i++) {
        const hasResult = lastCell.outputs.some(o => 
          o.items.some(item => new TextDecoder().decode(item.data).includes("15"))
        );
        if (lastCell.executionSummary?.success === true || hasResult) {
          lastSuccess = true;
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      assert.strictEqual(lastSuccess, true, "Last cell execution failed or timed out");
    } finally {
      if (fs.existsSync(testNbPath)) {
        fs.unlinkSync(testNbPath);
      }
    }
  });

  test("File-based Evaluation & InlayHints Test", async function (this: any) {
    this.timeout(60000);
    const testFilePath = path.join(os.tmpdir(), `test_eval_${Date.now()}.rkt`);
    const content = '#lang racket\n(+ 1 2)\n(define (f x) (* x x))\n(f 10)\n';
    fs.writeFileSync(testFilePath, content);

    try {
      const ext = vscode.extensions.getExtension("rkaramc.tools-scheme");
      assert.ok(ext, "Extension not found!");
      await ext.activate();

      const uri = vscode.Uri.file(testFilePath);
      const doc = await vscode.workspace.openTextDocument(uri);
      await vscode.window.showTextDocument(doc);

      await new Promise((resolve) => setTimeout(resolve, 5000));

      await vscode.commands.executeCommand("scheme.runEvaluation", uri);

      let hintsFound = false;
      const range = new vscode.Range(0, 0, 100, 0);
      for (let i = 0; i < 20; i++) {
        const hints = await vscode.commands.executeCommand<vscode.InlayHint[]>("vscode.executeInlayHintProvider", uri, range);
        if (hints && hints.length >= 2) {
          const hintTexts = hints.map(h => typeof h.label === 'string' ? h.label : h.label.map(l => l.value).join(''));
          if (hintTexts.some(t => t.includes('3')) && hintTexts.some(t => t.includes('100'))) {
            hintsFound = true;
            break;
          }
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(hintsFound, true, "Expected InlayHints (3 and 100) not found");
    } finally {
      if (fs.existsSync(testFilePath)) {
        fs.unlinkSync(testFilePath);
      }
    }
  });

  test("CodeLens on .rkt Files Test", async function (this: any) {
    this.timeout(60000);
    const testFilePath = path.join(os.tmpdir(), `test_codelens_${Date.now()}.rkt`);
    const content = '#lang racket\n(+ 1 2)\n\n(define (g x) (* x 2))\n(g 5)\n';
    fs.writeFileSync(testFilePath, content);

    try {
      const ext = vscode.extensions.getExtension("rkaramc.tools-scheme");
      assert.ok(ext, "Extension not found!");
      await ext.activate();

      const uri = vscode.Uri.file(testFilePath);
      const doc = await vscode.workspace.openTextDocument(uri);
      await vscode.window.showTextDocument(doc);

      await new Promise((resolve) => setTimeout(resolve, 5000));

      let lensesFound = false;
      for (let i = 0; i < 20; i++) {
        const lenses = await vscode.commands.executeCommand<vscode.CodeLens[]>("vscode.executeCodeLensProvider", uri);
        if (lenses && lenses.length >= 3) {
          const titles = lenses.map(l => l.command?.title).filter(t => t !== undefined);
          if (titles.some(t => t?.includes('Evaluate'))) {
            lensesFound = true;
            break;
          }
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(lensesFound, true, "Expected CodeLenses (Evaluate) not found");
    } finally {
      if (fs.existsSync(testFilePath)) {
        fs.unlinkSync(testFilePath);
      }
    }
  });

  test("Custom Commands (restartREPL, clearNamespace) Test", async function (this: any) {
    this.timeout(120000);
    const testFilePath = path.join(os.tmpdir(), `test_commands_${Date.now()}.rkt`);
    const content = '#lang racket\n(define val_establish 42)\nval_establish\n';
    fs.writeFileSync(testFilePath, content);

    try {
      const ext = vscode.extensions.getExtension("rkaramc.tools-scheme");
      assert.ok(ext, "Extension not found!");
      await ext.activate();

      const uri = vscode.Uri.file(testFilePath);
      const doc = await vscode.workspace.openTextDocument(uri);
      await vscode.window.showTextDocument(doc);
      
      const fullRange = new vscode.Range(0, 0, 1000, 0);
      await new Promise((resolve) => setTimeout(resolve, 5000));

      // 1. Establish state
      console.log('--- Step 1: Establish State (val_establish) ---');
      await vscode.commands.executeCommand("scheme.runEvaluation", uri);
      
      let hintFound = false;
      for (let i = 0; i < 20; i++) {
        const hints = await vscode.commands.executeCommand<vscode.InlayHint[]>("vscode.executeInlayHintProvider", uri, fullRange);
        if (hints && hints.some(h => (typeof h.label === 'string' ? h.label : h.label.map(l => l.value).join('')).includes('42'))) {
          hintFound = true;
          break;
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(hintFound, true, "Initial state not established");

      // 2. Clear Namespace
      console.log('--- Step 2: Clear Namespace ---');
      await vscode.commands.executeCommand("scheme.clearNamespace");
      
      const edit = new vscode.WorkspaceEdit();
      edit.replace(uri, fullRange, '#lang racket\nval_establish\n');
      await vscode.workspace.applyEdit(edit);
      await doc.save();
      await vscode.commands.executeCommand("scheme.runEvaluation", uri);

      let clearSuccess = false;
      for (let i = 0; i < 20; i++) {
        const diags = vscode.languages.getDiagnostics(uri);
        const diagMsg = diags.map(d => d.message).join(' | ');
        if (diagMsg.toLowerCase().includes('val_establish') && (diagMsg.toLowerCase().includes('undefined') || diagMsg.toLowerCase().includes('unbound'))) {
            clearSuccess = true;
            break;
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(clearSuccess, true, "val_establish should be undefined after clearNamespace");

      // 3. Restart REPL
      console.log('--- Step 3: Restart REPL (val_restart) ---');
      const edit3 = new vscode.WorkspaceEdit();
      edit3.replace(uri, fullRange, '#lang racket\n(define val_restart 100)\nval_restart\n');
      await vscode.workspace.applyEdit(edit3);
      await doc.save();
      
      await vscode.commands.executeCommand("scheme.runEvaluation", uri);
      
      let restartPrepFound = false;
      for (let i = 0; i < 20; i++) {
        const hints = await vscode.commands.executeCommand<vscode.InlayHint[]>("vscode.executeInlayHintProvider", uri, fullRange);
        if (hints && hints.some(h => (typeof h.label === 'string' ? h.label : h.label.map(l => l.value).join('')).includes('100'))) {
          restartPrepFound = true;
          break;
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(restartPrepFound, true, "State val_restart not established before restart");

      console.log('Triggering scheme.restartREPL...');
      await vscode.commands.executeCommand("scheme.restartREPL");
      await new Promise(resolve => setTimeout(resolve, 15000));

      const edit4 = new vscode.WorkspaceEdit();
      edit4.replace(uri, fullRange, '#lang racket\nval_restart\n');
      await vscode.workspace.applyEdit(edit4);
      await doc.save();
      await vscode.commands.executeCommand("scheme.runEvaluation", uri);

      let restartSuccess = false;
      for (let i = 0; i < 30; i++) {
        const diags = vscode.languages.getDiagnostics(uri);
        const hints = await vscode.commands.executeCommand<vscode.InlayHint[]>("vscode.executeInlayHintProvider", uri, fullRange);
        
        const diagMsg = diags.map(d => d.message).join(' | ');
        const hintLabels = hints ? hints.map(h => typeof h.label === 'string' ? h.label : h.label.map(l => l.value).join('')).join(' | ') : "NONE";
        
        console.log(`[DEBUG] Step 3 Loop ${i}: Diags: ${diagMsg} | Hints: ${hintLabels}`);

        const hasErrorDiag = diagMsg.toLowerCase().includes('val_restart') && (diagMsg.toLowerCase().includes('undefined') || diagMsg.toLowerCase().includes('unbound'));
        const hasErrorHint = hints && hints.some(h => {
            const label = (typeof h.label === 'string' ? h.label : h.label.map(l => l.value).join('')).toLowerCase();
            return label.includes('undefined') || label.includes('unbound') || (label.includes('val_restart') && !label.includes('100'));
        });

        if (hasErrorDiag || hasErrorHint) {
            restartSuccess = true;
            break;
        }
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
      assert.strictEqual(restartSuccess, true, "val_restart should be undefined after restartREPL");
    } finally {
      if (fs.existsSync(testFilePath)) {
        fs.unlinkSync(testFilePath);
      }
    }
  });
});
