#lang racket
(require racket/draw racket/class net/base64 racket/gui/base)
(require (except-in 2htdp/image make-color make-pen))

(define (image->base64-png img)
  (define-values (width height) (values (image-width img) (image-height img)))
  (define bmp (make-bitmap (inexact->exact (ceiling width)) (inexact->exact (ceiling height))))
  (define dc (make-object bitmap-dc% bmp))
  ;; snips draw themselves onto a dc%
  (send img draw dc 0 0 0 0 width height 0 0 '())
  (define out (open-output-bytes))
  (send bmp save-file out 'png)
  (bytes->string/utf-8 (base64-encode (get-output-bytes out))))

(define img (circle 10 "solid" "red"))
(displayln (image->base64-png img))
