# Scheme Toolbox: VS Code Extension Testing Guide

This guide provides a step-by-step walkthrough to verify the functionality of the Scheme Toolbox extension, from basic installation to advanced interactive workflows.

## Prerequisites
- **Racket** installed and in your `PATH`.
- **VS Code** with the Scheme Toolbox extension loaded (use `F5` in the extension development host).

---

## Phase 1: Basic Editor Features
Verify the core Language Server Protocol (LSP) features in standard `.rkt` files.

### ✔️ 1.1 Syntax Highlighting & Validation
1. ✔️ Create a new file `test.rktnb`.
2. ✔️ Add `#lang racket` at the top.
3. ✔️ Add a code cell and type `(define x 10)`.
4. **Expectation:**
   - ✔️ Keywords like `define` are highlighted. (Using `racket-notebook-cell` with grammar proxy)
   - ✔️ Autocomplete/Intellisense available.
   - ✔️ Diagnostics appear for errors (e.g. syntax errors). (Using our own LSP)
   - ✔️ No "Evaluate" CodeLens displayed. (Suppressed for notebook cells)

5. ✔️ Add a code cell and type `(define x "wrong")`.
6. **Expectation:**
   - ✔️ A diagnostic appears (yellow squiggle) indicating `x` is already defined in the module. (Downgraded to WARNING for notebooks)
   - *Note: Evaluation errors are also displayed in the cell output.*

### 1.2 Inlay Hints & Code Lenses
1. In `test.rkt`, write a top-level expression: `(+ 1 2)`.
2. **Expectation:** An "Evaluate" Code Lens appears above the line.
3. Hover over `+`.
4. **Expectation:** Documentation/type information (if available) appears in a hover.

---

## Phase 2: Standard Evaluation
Verify the non-notebook "Evaluate" flow.

### 2.1 Evaluate File
1. Open a `.rkt` file with multiple expressions.
2. Run the command `Scheme: Evaluate File` (Cmd/Ctrl + Shift + P).
3. **Expectation:** Results appear as Inlay Hints at the end of each line (e.g., `=> 3`).
4. **Expectation:** `stdout` output appears in the "Scheme Toolbox" Output channel.

### 2.2 Evaluate Selection
1. Highlight a specific S-expression (e.g., `(* 5 5)`).
2. Run `Scheme: Evaluate Selection`.
3. **Expectation:** Only the selected expression is evaluated; an inlay hint appears for that line.

---

## Phase 3: Notebook Workflows
Verify the "Pure Code" Notebook experience.

### 3.1 Opening as Notebook
1. Create a new file `test.rktnb`.
2. **Expectation:** The file opens with a notebook UI, splitting code into cells.
3. (Optional) Right-click an existing `.rkt` file and select **Open With...** -> **Scheme Notebook**.
   - *Note: Using `.rktnb` is recommended to avoid conflicts with other Racket extensions.*

### 3.2 Cell Execution
1. Create a cell with `(define y 100)`. Press `Shift + Enter`.
2. **Expectation:** The cell executes; a green checkmark appears.
3. Create a second cell with `(+ y 50)`. Run it.
4. **Expectation:** Output shows `150`. This verifies **persistent state** across cells.

### 3.3 Rich Media (Graphics)
1. Add a cell:
   ```racket
   (require 2htdp/image)
   (circle 50 "solid" "red")
   ```
2. Run the cell.
3. **Expectation:** A red circle is rendered directly in the notebook output.

### 3.4 Infinite Loop & Cancellation
1. Add a cell: `(let loop () (loop))`.
2. Run the cell.
3. **Expectation:** The cell shows a "running" state.
4. Click the **Stop/Interrupt** button on the cell.
5. **Expectation:** Execution stops; a "cancelled" error message appears.

---

## Phase 4: Advanced Scenarios

### 4.1 Coordinate Drift Stress Test
1. While a long-running cell is executing, rapidly type new lines of code or comments above it.
2. **Expectation:** Diagnostics and Inlay Hints should remain anchored to their correct expressions (this verifies the coordinate shifting logic).

### 4.2 Module Path Resolution
1. Create two files: `lib.rkt` (with a provided function) and `main.rkt`.
2. In `main.rkt`, use `(require "lib.rkt")`.
3. Evaluate a cell in `main.rkt` that calls the library function.
4. **Expectation:** The library is correctly resolved and evaluated.

### 4.3 Sandbox Protections
1. Run `(delete-file "important.txt")` (ensure the file doesn't exist or is safe).
2. **Expectation:** If sandboxing is active (in Notebook mode), the evaluation should fail with a security violation.

---

## Troubleshooting
- **Check Logs:** Open VS Code Output window and select `Scheme Toolbox` from the dropdown.
- **Reset Server:** Use `Scheme: Restart REPL` to clear all state and restart the Racket process.
