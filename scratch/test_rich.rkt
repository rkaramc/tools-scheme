#lang racket
(require racket/sandbox)
(define ev (make-evaluator 'racket/base))
(with-handlers ([exn:fail? (lambda (e) (displayln (exn-message e)))])
  (ev '(require 2htdp/image))
  (displayln (ev '(circle 10 "solid" "red"))))
