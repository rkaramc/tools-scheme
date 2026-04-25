#lang racket
(require racket/sandbox)
(define (check name)
  (with-handlers ([exn:fail? (lambda (e) (printf "~a: not found\n" name))])
    (define val (dynamic-require 'racket/sandbox name))
    (printf "~a: found\n" name)))

(check 'sandbox-network-guard)
