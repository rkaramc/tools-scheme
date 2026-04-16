#lang racket
(define test-file "utf16-test.rkt")
(with-output-to-file test-file #:exists 'replace
  (lambda ()
    (display "(define 😀 1)")
  ))

(define port (open-input-file test-file))
(port-count-lines! port)
; Read the whole script to find the emoji syntax
(define stx (read-syntax test-file port))

(printf "Whole stx: ~v\n" (syntax->datum stx))
; The syntax is (define 😀 1)
; We want to find the emoji one.
(define stx-list (syntax->list stx))
; define is (car stx-list)
; emoji is (cadr stx-list)
(define stx-emoji (cadr stx-list))

(printf "Emoji stx: content: ~v, line ~a, col ~a, pos ~a, span ~a\n" 
        (syntax->datum stx-emoji) (syntax-line stx-emoji) (syntax-column stx-emoji) (syntax-position stx-emoji) (syntax-span stx-emoji))

(define (utf16-length s)
  (for/fold ([len 0]) ([c (in-string s)])
    (+ len (if (> (char->integer c) #xFFFF) 2 1))))

(printf "UTF-16 length of '(define 😀 1)': ~a\n" (utf16-length "(define 😀 1)"))
(printf "Racket length of '(define 😀 1)': ~a\n" (string-length "(define 😀 1)"))

(delete-file test-file)
