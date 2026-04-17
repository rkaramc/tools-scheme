#lang racket
(require json)
(define test-file "drift-test.rkt")
(with-output-to-file test-file #:exists 'replace
  (lambda ()
    (display "(define x 1)\r\n(define y 2)\r\n(+ x y)")
  ))

(define (get-normalized-content path)
  (string-replace (file->string path) "\r\n" "\n"))

(define (simulate-syntax-end pos span line col content)
  (let ([p (open-input-string content)])
    (port-count-lines! p)
    (printf "String length: ~a\n" (string-length content))
    (printf "Setting file-position to ~a\n" (- pos 1))
    (with-handlers ([exn:fail? (lambda (e) (printf "Error setting position: ~a\n" (exn-message e)) (values line col))])
      (file-position p (- pos 1))
      (set-port-next-location! p line col pos)
      (let ([read-stx (read-string span p)])
        (printf "Read string from port: ~v\n" read-stx)
        (define-values (l c p-end) (port-next-location p))
        (values l c)))))

(printf "Testing CRLF drift reproduction...\n")

(define port (open-input-file test-file))
(port-count-lines! port)
(define stx1 (read-syntax test-file port))
(printf "stx1: line ~a, col ~a, pos ~a, span ~a content: ~v\n" 
        (syntax-line stx1) (syntax-column stx1) (syntax-position stx1) (syntax-span stx1) (syntax->datum stx1))

(define stx2 (read-syntax test-file port))
(printf "stx2: line ~a, col ~a, pos ~a, span ~a content: ~v\n" 
        (syntax-line stx2) (syntax-column stx2) (syntax-position stx2) (syntax-span stx2) (syntax->datum stx2))

(define normalized (get-normalized-content test-file))
(printf "Normalized length: ~a (Original: ~a)\n" (string-length normalized) (string-length (file->string test-file)))

(printf "\n--- Using Normalized Content ---\n")
(define-values (l c) (simulate-syntax-end (syntax-position stx2) (syntax-span stx2) (syntax-line stx2) (syntax-column stx2) normalized))
(printf "Calculated end for stx2: line ~a, col ~a\n" l c)

(printf "\n--- Using Original Content ---\n")
(define-values (l2 c2) (simulate-syntax-end (syntax-position stx2) (syntax-span stx2) (syntax-line stx2) (syntax-column stx2) (file->string test-file)))
(printf "Calculated end for stx2: line ~a, col ~a\n" l2 c2)

(delete-file test-file)
