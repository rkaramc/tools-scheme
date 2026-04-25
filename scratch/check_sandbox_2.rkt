#lang racket
(require racket/sandbox)
(with-handlers ([exn:fail:syntax? (lambda (e) (displayln "Syntax error during check"))])
  (eval '(displayln sandbox-network-allowed?)))
