#lang racket
(define p (open-input-string "(require racket/pretty) (define atom? (lambda (x) #t)) (atom? 1)"))
(require "eval-shim.rkt")
; Mocking the REPL behavior since eval-shim.rkt doesn't export functions easily due to module+ main, 
; but I can probably just call evaluate-string-content if I wrap it or use a separate test.
