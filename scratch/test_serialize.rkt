#lang racket
(require racket/draw racket/class net/base64)

(define (image->base64-png img)
  (define width (send img get-width))
  (define height (send img get-height))
  (define bmp (make-bitmap (inexact->exact (ceiling width)) (inexact->exact (ceiling height))))
  (define dc (make-object bitmap-dc% bmp))
  (send img draw dc 0 0 0 0 width height 0 0 '())
  (define out (open-output-bytes))
  (send bmp save-file out 'png)
  (base64-encode (get-output-bytes out)))

(require 2htdp/image)
(define img (circle 10 "solid" "red"))
(displayln (image->base64-png img))
