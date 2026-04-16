#lang racket
(require json)

;; Helper to run a command and capture its JSON output
(define (run-shim-repl input-expr)
  (define-values (process out in err)
    (subprocess #f #f #f (find-executable-path "racket") "lsp/src/eval-shim.rkt" "--repl"))
  
  (displayln (jsexpr->string input-expr) in)
  (flush-output in)
  (close-output-port in)
  
  (let loop ([results '()])
    (define line (read-line out))
    (cond
      [(eof-object? line) 
       (begin (close-input-port out) (close-input-port err) results)]
      [(string=? line "READY") (loop results)]
      [else 
       (with-handlers ([exn:fail? (lambda (e) (loop results))])
         (loop (append results (list (string->jsexpr line)))))])))

;; ---- TEST CASE: Multi-line Coordinate Ranges via File ----
(printf "Running ts-z8w Multi-line Coordinate File Test...\n")

(define test-file "d:/source/tools-scheme/lsp/src/test-coords-smart.rkt")
(with-output-to-file test-file #:exists 'replace
  (lambda ()
    (display "(+ 1\n   2)")
  ))

;; Send evaluate-file command (but wait, in the REPL we pass 'content' as the path)
;; Actually, the REPL currently ONLY supports EvaluateStringContent.
;; I'll just run the shim in FILE mode.

(define (run-shim-file path)
  (define-values (process out in err)
    (subprocess #f #f #f (find-executable-path "racket") "lsp/src/eval-shim.rkt" path))
  (close-output-port in)
  (let loop ([results '()])
    (define line (read-line out))
    (if (eof-object? line) results (loop (append results (list (string->jsexpr line)))))))

(define results-json (run-shim-file test-file))

(printf "Shim Results count: ~a\n" (length results-json))
(for ([res results-json])
  (printf "Result: start ~a:~a, end ~a:~a, val ~a\n" 
          (hash-ref res 'line) (hash-ref res 'col) 
          (hash-ref res 'end_line) (hash-ref res 'end_col)
          (hash-ref res 'result)))

(unless (= (length results-json) 1)
  (delete-file test-file)
  (error "Expected 1 result, got" (length results-json)))

(define res (list-ref results-json 0))

(delete-file test-file)

;; (+ 1
;;    2)
;; starts at 1:0.
;; ends at 2:4.

(unless (and (= (hash-ref res 'line) 1)
             (= (hash-ref res 'col) 0)
             (= (hash-ref res 'end_line) 2)
             (= (hash-ref res 'end_col) 5))
  (error "Incorrect coordinates reported!" res))

(printf "Test Passed!\n")
