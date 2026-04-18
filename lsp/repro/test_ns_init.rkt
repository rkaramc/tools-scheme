#lang racket

(define (test-lang-ns-eval)
  (define m-ns (make-base-empty-namespace))
  (parameterize ([current-namespace m-ns])
    (namespace-require 'racket)
    (eval '(define x 10))
    (printf "x is ~a\n" (eval 'x))))

(test-lang-ns-eval)
