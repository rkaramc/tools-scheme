#lang racket
(require racket/sandbox racket/class racket/snip)
(define ev (parameterize ([sandbox-namespace-specs
                           (list make-base-namespace
                                 'racket/class
                                 'racket/snip)])
             (make-evaluator 'racket/base)))
(with-handlers ([exn:fail? (lambda (e) (displayln (exn-message e)))])
  (ev '(require 2htdp/image))
  (define val (ev '(circle 10 "solid" "red")))
  (printf "object?: ~a\n" (object? val))
  (printf "is-a? snip%: ~a\n" (is-a? val snip%)))