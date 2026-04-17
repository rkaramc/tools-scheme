#lang racket
(require json)

(printf "Running ts-495 CRLF Drift Position Test...\n")

(define test-file "d:/source/tools-scheme/lsp/src/test-crlf-run.rkt")
(with-output-to-file test-file #:exists 'replace
  (lambda ()
    (for ([i (in-range 1 501)]) ;; 500 lines to be ABSOLUTELY sure we drift enough
      (printf "(+ ~a ~a)\r\n" i i))
  ))

;; We'll use subprocess to run the shim and capture its stderr
(define-values (process out in err)
  (subprocess #f #f #f (find-executable-path "racket") "lsp/src/eval-shim.rkt" test-file))

(close-output-port in)
(define results (port->lines out))
(define errors (port->lines err))

(printf "Shim Results count: ~a\n" (length results))
(printf "Shim Errors count: ~a\n" (length errors))
(for ([e errors]) (printf "Error: ~a\n" e))

(delete-file test-file)

;; If normalization is happening and causing drift:
;; Original: 500 lines * approx 12 chars/line = 6000 chars.
;; CRLF: 500 * (\r\n) = 1000 bytes.
;; LF: 500 * (\n) = 500 bytes.
;; Drift = 500 characters.
;; syntax-position for line 500 will be ~6000.
;; get-syntax-end will try to setting file-position to 5999 in a 5500-char normalized string.
;; This WILL throw an error about 'file-position' being beyond the end of the port.

(if (not (empty? errors))
    (begin
      (printf "Test Passed: Drift error detected as expected!\n")
      (exit 0))
    (begin
      (printf "Test Failed: No drift error detected. Normalized content might be accidentally consistent or large enough.\n")
      (exit 1)))
