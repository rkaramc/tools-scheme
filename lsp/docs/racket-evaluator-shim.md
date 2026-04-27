# Racket Evaluator Shim — Architecture Overview

**Source**: [`lsp/src/eval-shim.rkt`](../src/eval-shim.rkt)

The eval-shim is a Racket program that runs as a child process of the LSP server.
It evaluates Racket code and emits structured JSON results over stdout. The Rust
LSP server communicates with it via stdin/stdout using a line-delimited JSON protocol.

## Global State

```
real-stdout          = saved reference to actual stdout (before sandbox redirects it)
file-content-cache   = HashMap<source, content>     # LRU cache, max 100 entries
cache-access-log     = HashMap<source, counter>     # access timestamps for eviction
document-evaluators  = HashMap<uri, sandbox>        # per-document sandbox evaluators
current-eval-thread  = thread handle (mutable)
current-evaluator    = sandbox handle (mutable)
```

## Utility Functions

```
make-range(line, col, end_line, end_col, span, pos):
  return { line, col, end_line, end_col, span, pos }   # with defaults for nulls

truncate-string(str, limit):
  if len(str) > limit: return str[0..limit] + "... [truncated]"
  else: return str

uri-decode(str):
  scan bytes; replace %XX sequences with decoded byte
  return UTF-8 string

uri->path(uri):
  if not starts-with "file:///": return #f
  strip prefix, percent-decode, normalize slashes on Windows
  return native path object
```

## Content Cache (LRU)

```
cache-content!(content, source):
  increment global counter
  store content and access timestamp
  if cache.size > 100:
    find entry with lowest access counter → evict it
  return content

get-cached-content(path):
  if path is #f or symbol: return ""
  update access counter
  return cache[path] or read-file-and-cache(path)
```

## Syntax Coordinate Helpers

```
get-syntax-end(stx):
  # Placeholder coordinates — Rust side recalculates precisely
  # via recalculate_from_byte_pos using span + raw text buffer
  return (line, col + span)

get-exn-location(exception, stx):
  if exception has srcloc info: extract line, col, span, pos from it
  else: fall back to syntax object coordinates
```

## Output / Display

```
display-result(range, value, is_error=false, output=""):
  format value as string (or "Rich media" for snips)
  truncate result and output to MAX_OUTPUT_SIZE (10000)
  if value is a snip:
    render to bitmap → PNG → base64
    emit JSON { type: "rich", mime: "image/png", data: base64 } to real-stdout
  emit JSON { ...range, result, is_error, output } to real-stdout

make-streaming-port(type, uri):
  return custom output-port that, on each write:
    emits JSON { type: "output", stream: type, data: chunk, uri } to real-stdout
```

## Core Evaluation Engine

```
for-each-syntax(port, source, callback):
  loop:
    save file position
    try: read-syntax from port
      if EOF: stop
      call callback(syntax-object)
    catch read error:
      emit error result with location
      skip line, continue loop

evaluate-single-form(stx, namespace):
  try:
    capture stdout to temp port
    eval(stx) in namespace
    captured = get captured output
    if result is not void: emit result with range
    elif captured is not empty: emit void + captured output
  catch: emit error with location

eval-module-body-form(form, module-ns, file-dir):
  if form is (require "relative-path"):
    resolve each relative path against file-dir
    namespace-require each resolved spec
  else:
    evaluate-single-form(form, module-ns)

evaluate-port(port, source, namespace):
  for each syntax object in port:
    if it's a (module name lang (module-begin body ...)):
      # Module form: decompose and eval body forms individually.
      # Avoids ts-h31 namespace-mismatch bug from pre-expanding.
      try: namespace-require(lang)
      catch: skip lang, eval body forms anyway
      for each body form: eval-module-body-form(form, ns, file-dir)
    else:
      # Plain top-level form
      evaluate-single-form(stx, namespace)
```

## Entry Points

```
evaluate-file(path):
  if path == "-":
    copy stdin to temp file; use temp file as target
  open file, enable line counting
  set current-directory and load-relative-directory to file's parent
  create base namespace (attach racket/class, racket/snip)
  evaluate-port(file, path, namespace)
  if stdin mode: delete temp file

parse-string-content(content, uri):
  cache content
  for each syntax object:
    if module form: recurse into #%module-begin body
    else: emit JSON { ...range, type: "range" } to real-stdout

evaluate-string-content(content, uri):
  get-or-create sandbox evaluator for URI
  cache content
  consume #lang header if present (without consuming body)
  for each syntax object:
    run in sandbox evaluator
    if result is not void: emit result with range

get-evaluator(uri, content):
  if evaluator exists for URI: return it
  else:
    parse #lang from content (fallback: racket/base)
    normalize lang (procedure → 'racket)
    create sandboxed evaluator with:
      - streaming stdout/stderr ports
      - 128MB memory limit, 15s eval timeout
      - current-directory set to file's parent
      - allow-read for file's directory
    cache and return evaluator
```

## REPL Loop (`--repl` mode)

```
run-repl():
  loop: read JSON line from stdin
    match type:
      "evaluate":
        spawn thread → evaluate-string-content(content, uri)
        thread emits "READY" when done
        store thread + evaluator handles for cancellation

      "cancel-evaluation":
        break-evaluator(current-evaluator)   # sends break signal to sandbox

      "parse":
        parse-string-content(content, uri)
        emit "READY"

      "clear-namespace":
        remove evaluator for URI from document-evaluators
        emit "READY"

      else:
        log "Unknown REPL command type" to stderr
        emit "READY"

    catch any JSON/eval error:
      emit error result, emit "READY", continue loop
```

## Main Entry Point

```
main:
  parse command line:
    --repl     → run-repl(), exit
    <filename> → evaluate-file(filename)
```

## Architectural Notes

### Two Execution Models

File mode uses a plain namespace (`make-base-namespace`); REPL mode uses
`racket/sandbox` with resource limits. These are separate code paths
(`evaluate-port` vs `evaluate-string-content`).

### READY Protocol

The Rust side reads stdout line-by-line and treats `READY` as the
end-of-response sentinel. All JSON results are emitted before it.

### Per-Document Isolation

`document-evaluators` maps each URI to its own sandbox instance. State
persists across evaluations of the same URI. `clear-namespace` removes
the evaluator, forcing a fresh sandbox on next evaluation.

### Module Decomposition (ts-h31)

Instead of expanding `#lang` modules (which causes namespace-mismatch
errors due to the `|expanded module|` scope stamp), the shim reads raw
syntax and evals body forms individually against a namespace that has the
language required into it.

### Relative Require Resolution (ts-k2w)

In a plain namespace, relative-string require paths have no reference
point. The shim detects `(require "relative.rkt")` forms and resolves
them to absolute paths using the file's parent directory before calling
`namespace-require`.

### Coordinate Accuracy

`get-syntax-end` provides simplified placeholder coordinates. The Rust
side (`recalculate_from_byte_pos` in `server.rs`) recalculates accurate
multi-line boundaries and UTF-16 code unit offsets using the `span`,
`pos`, and raw text buffer.
