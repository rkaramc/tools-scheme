#lang racket
(require racket/snip racket/class)
(require (except-in 2htdp/image make-color make-pen))
(define val (circle 10 "solid" "red"))
(printf "is-a? snip%: ~a\n" (is-a? val snip%))
(printf "class: ~a\n" (object-interface val))
