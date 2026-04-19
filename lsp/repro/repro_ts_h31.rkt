#lang racket

; Repro for ts-h31: recursive function in a #lang file causes
; "namespace mismatch; cannot locate module instance" errors.
; The self-reference (lat? (cdr x)) inside lat?'s body is stamped
; with the expanded module's scope, which is lost when each form
; is eval'd individually outside the module.

(define atom? (lambda (x) (and (not (pair? x)) (not (null? x)))))

(define (lat? x)
  (cond
    ((null? x) #t)
    ((atom? (car x)) (lat? (cdr x)))  ; <-- namespace mismatch here
    (else #f)))

(lat? '(a b c))     ; => #t
(lat? '(a (b) c))   ; => #f
