#lang racket

(require json racket/exn)

(define real-stdout (current-output-port))

;; Current one-shot evaluation logic
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
                                     (define-values (l c) (get-exn-location e stx target-path))
                                     (display-result l c 0 e #t "")
                                     (loop))])
          
          (define expanded (expand stx))
          (syntax-case expanded (module)
            [(module name lang (mb . body))
             (let ([m-name (syntax-e #'name)])
               (eval expanded)
               (dynamic-require `(quote ,m-name) #f)
               (define ns (module->namespace `(quote ,m-name)))
               (for ([form (syntax->list #'body)])
                 (evaluate-single-form form ns target-path)))]
            [_ 
             (evaluate-single-form stx (current-namespace) target-path)])
          (loop)))))
  
  (when (string=? path "-") (delete-file target-path)))

(define (evaluate-single-form stx ns target-path)
  (with-handlers ([exn:fail? (lambda (e) 
                               (define-values (l c) (get-exn-location e stx target-path))
                               (display-result l c 0 e #t ""))])
    (define capture-port (open-output-string))
    (define result
      (parameterize ([current-output-port capture-port]
                     [current-namespace ns])
        (eval stx)))
    (define captured (get-output-string capture-port))
    
    (define-values (end-line end-col) (get-syntax-end stx target-path))

    (cond
      [(not (void? result))
       (display-result end-line end-col 0 result #f captured)]
      [(not (string=? captured ""))
       (display-result end-line end-col 0 'void #f captured)])))

;; Persistent REPL logic
(define (run-repl)
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t]
                 [current-namespace (make-base-namespace)])
    (let loop ()
      (define input (read-line))
      (unless (eof-object? input)
        (with-handlers ([exn:fail? (lambda (e)
                                     (display-result 1 0 0 e #t "")
                                     (displayln "READY" real-stdout)
                                     (flush-output real-stdout)
                                     (loop))])
          (let ([json-input (string->jsexpr input)])
            (define type (hash-ref json-input 'type))
            (define content (hash-ref json-input 'content))
            
            (cond
              [(string=? type "evaluate")
               (evaluate-string-content content)]
              [else
               (eprintf "Unknown REPL command type: ~a\n" type)]))
          (displayln "READY" real-stdout)
          (flush-output real-stdout)
          (loop))))))

(define (evaluate-string-content content)
  (define port (open-input-string content))
  (port-count-lines! port)
  (let loop ()
    (define stx (read-syntax 'repl port))
    (unless (eof-object? stx)
      (with-handlers ([exn:fail? (lambda (e)
                                   (define-values (l c) (get-exn-location e stx 'repl))
                                   (display-result l c 0 e #t "")
                                   (loop))])
        (define expanded (expand stx))
        (syntax-case expanded (module)
          [(module name lang (mb . body))
           (let ([m-name (syntax-e #'name)])
             (eval expanded)
             (dynamic-require `(quote ,m-name) #f)
             (define ns (module->namespace `(quote ,m-name)))
             (for ([form (syntax->list #'body)])
               (evaluate-single-form form ns 'repl)))]
          [_
           (evaluate-single-form stx (current-namespace) 'repl)])
        (loop)))))


(define file-content-cache (make-hash))
(define (get-normalized-content path)
  (if (symbol? path)
      "" ;; No content for REPL symbols
      (hash-ref! file-content-cache path
                 (lambda ()
                   (string-replace (file->string path) "\r\n" "\n")))))

(define (get-exn-location e stx target-path)
  (let ([loc (and (exn:srclocs? e) 
                  (pair? ((exn:srclocs-accessor e) e))
                  (car ((exn:srclocs-accessor e) e)))])
    (if (and loc (srcloc-line loc) (srcloc-column loc))
        (values (srcloc-line loc) (srcloc-column loc))
        (get-syntax-end stx target-path))))

(define (get-syntax-end stx target-path)
  (let ([pos (syntax-position stx)]
        [span (syntax-span stx)])
    (if (and (not (symbol? target-path)) pos span)
        (let ([p (open-input-string (get-normalized-content target-path))])
          (port-count-lines! p)
          (read-string (- pos 1) p) ;; Skip to start
          (read-string span p)      ;; Read the actual syntax
          (define-values (l c p-end) (port-next-location p))
          (values l c))
        (values (or (syntax-line stx) 1) 
                (+ (or (syntax-column stx) 0) (or span 0))))))

(define (display-result line col span val is-error output)
  (define base
    (hasheq 'line (or line 1)
            'col (or col 0)
            'result (if (exn? val) (exn-message val) (format "~a" val))
            'is_error is-error
            'output output))
  (displayln (jsexpr->string base) real-stdout)
  (flush-output real-stdout))

(module+ main
  (require racket/cmdline)
  (command-line
   #:program "eval-shim"
   #:once-each
   [("--repl") "Run in persistent REPL mode" (run-repl) (exit 0)]
   #:args (filename)
   (evaluate-file filename)))
