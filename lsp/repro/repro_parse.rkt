#lang racket
(define stx
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (read-syntax 'src (open-input-string "#lang racket\n(+ 1 2)\n(define x 3)"))))

(printf "stx: ~v\n" stx)

(syntax-case stx (module)
  [(module name lang (mb . body))
   (begin
     (printf "body: ~v\n" (syntax->list #'body))
     (for ([form (syntax->list #'body)])
       (printf "form line: ~a\n" (syntax-line form))))]
  [_ (printf "not a module\n")])

(printf "Expanding...\n")
(with-handlers ([exn:fail? (lambda (e) (printf "Expand failed: ~a\n" (exn-message e)))])
  (parameterize ([current-namespace (make-base-namespace)])
    (expand stx)))
