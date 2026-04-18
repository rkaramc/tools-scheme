#lang racket
(define (test s)
  (printf "Testing ~v\n" s)
  (define p (open-input-string s))
  (port-count-lines! p)
  (let loop ()
    (define stx (read-syntax "source" p))
    (unless (eof-object? stx)
      (printf "  stx: ~v line: ~a col: ~a pos: ~a span: ~a\n" 
              (syntax-e stx)
              (syntax-line stx) 
              (syntax-column stx) 
              (syntax-position stx) 
              (syntax-span stx))
      (loop))))

(test "(a\r\n b)")
(test "a\r\nb")
(test "a\nb")
