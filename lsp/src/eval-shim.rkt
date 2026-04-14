#lang racket

(require json racket/exn)

(define real-stdout (current-output-port))

(define (evaluate-file path)
  (define target-path
    (if (string=? path "-")
        (let ([tmp (make-temporary-file "eval-shim-~a.rkt")])
          (with-output-to-file tmp #:exists 'replace
            (lambda () (copy-port (current-input-port) (current-output-port))))
          tmp)
        path))

  (define port (open-input-file target-path))
  (port-count-lines! port)
  
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t]
                 [current-namespace (make-base-namespace)])
    (let loop ()
      (define stx (read-syntax target-path port))
      (unless (eof-object? stx)
        (with-handlers ([exn:fail? (lambda (e) 
                                     (display-result (syntax-line stx) 
                                                     (syntax-column stx)
                                                     (syntax-span stx)
                                                     e #t "")
                                     (loop))])
          
          (define expanded (expand stx))
          (syntax-case expanded (module)
            [(module name lang (mb . body))
             (let ([m-name (syntax-e #'name)])
               (eval expanded)
               (dynamic-require `(quote ,m-name) #f)
               (define ns (module->namespace `(quote ,m-name)))
               (for ([form (syntax->list #'body)])
                 (evaluate-single-form form ns)))]
            [_ 
             (evaluate-single-form stx (current-namespace))])
          (loop)))))
  
  (when (string=? path "-") (delete-file target-path)))

(define (evaluate-single-form stx ns)
  (with-handlers ([exn:fail? (lambda (e) 
                               (display-result (syntax-line stx) 
                                               (syntax-column stx)
                                               (syntax-span stx)
                                               e #t ""))])
    (define capture-port (open-output-string))
    (define result
      (parameterize ([current-output-port capture-port]
                     [current-namespace ns])
        (eval stx)))
    (define captured (get-output-string capture-port))
    (cond
      [(not (void? result))
       (display-result (syntax-line stx) 
                       (syntax-column stx)
                       (syntax-span stx)
                       result #f captured)]
      [(not (string=? captured ""))
       (display-result (syntax-line stx) 
                       (syntax-column stx)
                       (syntax-span stx)
                       'void #f captured)])))

(define (display-result line col span val is-error output)
  (define end-col (+ (or col 0) (or span 0)))
  (define base
    (hasheq 'line (or line 1)
            'col end-col
            'result (if (exn? val) (exn-message val) (format "~a" val))
            'is_error is-error
            'output output))
  (displayln (jsexpr->string base) real-stdout))

(module+ main
  (command-line
   #:args (filename)
   (evaluate-file filename)))
