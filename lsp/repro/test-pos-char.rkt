#lang racket
(define test-file "drift-test.rkt")
(with-output-to-file test-file #:exists 'replace
  (lambda ()
    (display "(define x 1)\r\n(define y 2)")
  ))

(define content (file->string test-file))
(printf "File String length: ~a\n" (string-length content))

(define port (open-input-file test-file))
(port-count-lines! port)
(define stx1 (read-syntax test-file port))
(define stx2 (read-syntax test-file port))

(printf "stx2: line ~a, col ~a, pos ~a, span ~a content: ~v\n" 
        (syntax-line stx2) (syntax-column stx2) (syntax-position stx2) (syntax-span stx2) (syntax->datum stx2))

(printf "Character at index (pos-1) ~a: ~v\n" (- (syntax-position stx2) 1) (string-ref content (- (syntax-position stx2) 1)))

(delete-file test-file)
