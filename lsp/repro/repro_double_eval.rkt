#lang racket

(define (test-module-eval)
  (define ns (make-base-namespace))
  (parameterize ([current-namespace ns])
    (define stx
      (expand
       '(module m racket
          (define x 1)
          x)))
    
    (printf "--- Evaluating full module ---\n")
    (eval stx)
    (printf "Module evaluated. Now trying to evaluate (define x 1) again in module namespace...\n")
    
    (define m-ns (module->namespace ''m))
    (with-handlers ([exn:fail? (lambda (e) (printf "Caught expected error during double-eval: ~a\n" (exn-message e)))])
      (parameterize ([current-namespace m-ns])
        (eval '(define x 2))))))

(test-module-eval)
