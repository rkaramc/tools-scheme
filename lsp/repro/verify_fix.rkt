#lang racket
(require json)

(define (test-fix)
  (define-values (process stdout stdin stderr)
    (subprocess #f #f #f (find-executable-path "racket") "lsp/src/eval-shim.rkt" "--repl"))
  
  (define (send-json j)
    (displayln (jsexpr->string j) stdin)
    (flush-output stdin))
  
  (define (read-until-ready)
    (let loop ()
      (define line (read-line stdout))
      (printf "SHIM: ~a\n" line)
      (unless (or (eof-object? line) (string=? line "READY"))
        (loop))))

  ;; Scenario 2: #lang racket with a define
  (printf "--- Scenario 2 (Pass 1) ---\n")
  (send-json (hasheq 'type "evaluate" 
                     'uri "file:///test.rkt" 
                     'content "#lang racket\n(define x 1)\nx"))
  (read-until-ready)
  
  ;; Scenario 2 again: should still work (redefinition)
  (printf "\n--- Scenario 2 (Pass 2) ---\n")
  (send-json (hasheq 'type "evaluate" 
                     'uri "file:///test.rkt" 
                     'content "#lang racket\n(define x 2)\nx"))
  (read-until-ready)

  (close-output-port stdin)
  (subprocess-wait process))

(test-fix)
