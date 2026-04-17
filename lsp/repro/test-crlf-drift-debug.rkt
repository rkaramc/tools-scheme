#lang racket
(require json)

(define (run-shim-repl-debug input-expr)
  (define-values (process out in err)
    (subprocess #f #f #f (find-executable-path "racket") "lsp/src/eval-shim.rkt" "--repl"))
  
  (displayln (jsexpr->string input-expr) in)
  (flush-output in)
  (close-output-port in)
  
  (printf "--- Stdout ---\n")
  (define out-lines (port->lines out))
  (for ([l out-lines]) (printf "~a\n" l))

  (printf "--- Stderr ---\n")
  (define err-lines (port->lines err))
  (for ([l err-lines]) (printf "~a\n" l))

  (close-input-port out)
  (close-input-port err)
  (void))

(printf "Running ts-495 CRLF Drift Debug Test...\n")

(define input-expr 
  (hasheq 'type "evaluate" 
          'content "(define x 1)\r\n(define y 2)"))

(run-shim-repl-debug input-expr)
