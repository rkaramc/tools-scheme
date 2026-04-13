#lang racket

(require json racket/exn)

;; The shim takes a filename as a command-line argument.
(define (evaluate-file path)
  (define-values (port-orig) (open-input-file path))
  (port-count-lines! port-orig)
  (define first-line (read-line port-orig))
  (define port
    (if (and (string? first-line) (string-prefix? first-line "#lang"))
        port-orig
        (begin
          (close-input-port port-orig)
          (let ([p (open-input-file path)])
            (port-count-lines! p)
            p))))

  (parameterize ([current-namespace (make-base-namespace)]
                 [read-accept-reader #t]
                 [read-accept-lang #t])
    (let loop ()
      (define stx (read-syntax path port))
      (unless (eof-object? stx)
        (with-handlers ([exn:fail? (lambda (e) 
                                     (display-result (syntax-line stx) 
                                                     (syntax-column stx)
                                                     e #t)
                                     (loop))])
          (define result (eval stx))
          ;; We only display results for non-void expressions (like definitions)
          (unless (void? result)
            (display-result (syntax-line stx) 
                            (syntax-column stx)
                            result #f))
          (loop))))))

(define (display-result line col val is-error)
  (define output 
    (hasheq 'line line
            'col col
            'result (if (exn? val) (exn-message val) (format "~a" val))
            'is_error is-error))
  (displayln (jsexpr->string output)))

(module+ main
  (command-line
   #:args (filename)
   (evaluate-file filename)))
