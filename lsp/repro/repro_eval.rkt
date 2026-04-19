#lang racket
(define stx
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (read-syntax 'src (open-input-string "#lang racket\n(+ 1 2)\n(define x 3)"))))

(define expanded
  (parameterize ([current-namespace (make-base-namespace)])
    (expand stx)))

(let ([m-ns (make-base-namespace)])
  (parameterize ([current-namespace m-ns])
    ;; The syntax-case using literal `module` fails, so evaluate-port uses the fallback!
    (define l (syntax->list expanded))
    (if (and l (>= (length l) 4) (eq? (syntax-e (car l)) 'module))
        (let* ([lang (caddr l)]
               [body-stx (cadddr l)]
               [body-list (syntax->list body-stx)])
          (namespace-require (syntax->datum lang))
          (for ([form (cdr body-list)])
            (with-handlers ([exn:fail? (lambda (e) (printf "Eval failed: ~a\n" (exn-message e)))])
              (eval form))))
        (printf "Not a module\n"))))
