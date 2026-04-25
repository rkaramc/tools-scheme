#lang racket
(require racket/sandbox)
(for ([x (module->exports 'racket/sandbox)])
  (for ([y (cdr x)])
    (define name (symbol->string (car y)))
    (when (string-prefix? name "sandbox-")
      (displayln name))))
