#lang racket

(define ns (make-base-namespace))
(parameterize ([current-namespace ns])
  (printf "--- Scenario 3: Define atom? at top level ---\n")
  (eval '(define atom? (lambda (x) #t)))
  (printf "atom? defined: ~a\n" (eval '(atom? 1)))
  
  (printf "\n--- Scenario 2: Evaluate a module that also defines atom? ---\n")
  (with-handlers ([exn:fail? (lambda (e) (printf "Caught expected error: ~a\n" (exn-message e)))])
    (eval '(module anonymous-module racket
             (define atom? (lambda (x) #f))
             (atom? 1)))))
