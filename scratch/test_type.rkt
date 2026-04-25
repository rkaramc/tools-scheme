#lang racket
(require racket/sandbox racket/draw racket/class)
(define ev (make-evaluator 'racket/base))
(ev '(require 2htdp/image))
(define result (ev '(circle 10 "solid" "red")))
(displayln (is-a? result (dynamic-require 'racket/draw 'bitmap%)))
(displayln result)
