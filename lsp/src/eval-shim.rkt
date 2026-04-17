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
      (define pos (file-position port))
      (with-handlers ([exn:fail? (lambda (e)
                                   (define-values (l c end-c) (get-exn-location e #f target-path))
                                   (display-result (make-range l c l end-c) e #:is-error #t)
                                   (file-position port pos)
                                   (read-line port)
                                   (loop))])
        (define stx (read-syntax target-path port))
        (unless (eof-object? stx)
          (with-handlers ([exn:fail? (lambda (e) 
                                       (define-values (l c end-c) (get-exn-location e stx target-path))
                                       (display-result (make-range l c l end-c) e #:is-error #t)
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
            (loop))))))
  
  (when (string=? path "-") (delete-file target-path)))

(define (evaluate-single-form stx ns target-path)
  (with-handlers ([exn:fail? (lambda (e) 
                               (define-values (l c end-c) (get-exn-location e stx target-path))
                               (display-result (make-range l (or (syntax-column stx) 0) l end-c) e #:is-error #t))])
    (define capture-port (open-output-string))
    (define result
      (parameterize ([current-output-port capture-port]
                     [current-namespace ns])
        (eval stx)))
    (define captured (get-output-string capture-port))
    
    (define-values (end-line end-col) (get-syntax-end stx target-path))
    (define start-line (or (syntax-line stx) 1))
    (define start-col (or (syntax-column stx) 0))

    (cond
      [(not (void? result))
       (display-result (make-range start-line start-col end-line end-col) result #:output captured)]
      [(not (string=? captured ""))
       (display-result (make-range start-line start-col end-line end-col) 'void #:output captured)])))

;; Persistent REPL logic
(define document-namespaces (make-hash))

(define (run-repl)
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (let loop ()
      (define input (read-line))
      (unless (eof-object? input)
        (with-handlers ([exn:fail? (lambda (e)
                                     (display-result (make-range 1 0 1 0) e #:is-error #t)
                                     (displayln "READY" real-stdout)
                                     (flush-output real-stdout)
                                     (loop))])
          (let ([json-input (string->jsexpr input)])
            (define type (hash-ref json-input 'type))
            (define uri (hash-ref json-input 'uri #f))
            
            (cond
              [(string=? type "evaluate")
               (evaluate-string-content (hash-ref json-input 'content) uri)]
              [(string=? type "parse")
               (parse-string-content (hash-ref json-input 'content) uri)]
              [(string=? type "clear-namespace")
               (when uri
                 (hash-remove! document-namespaces uri))]
              [else
               (eprintf "Unknown REPL command type: ~a\n" type)]))
          (displayln "READY" real-stdout)
          (flush-output real-stdout)
          (loop))))))

(define (parse-string-content content uri)
  (define source (or uri 'parser))
  (hash-set! file-content-cache source content)
  (define port (open-input-string content))
  (port-count-lines! port)
  (let loop ()
    (define pos (file-position port))
    (with-handlers ([exn:fail? (lambda (e)
                                 (define-values (l c end-c) (get-exn-location e #f source))
                                 (display-result (make-range l c l end-c) e #:is-error #t)
                                 (file-position port pos)
                                 (read-line port)
                                 (loop))])
      (define stx (read-syntax source port))
      (unless (eof-object? stx)
        (define-values (end-line end-col) (get-syntax-end stx source))
        (define start-line (or (syntax-line stx) 1))
        (define start-col (or (syntax-column stx) 0))
        (define range (hash-set (make-range start-line start-col end-line end-col) 'type "range"))
        (displayln (jsexpr->string range) real-stdout)
        (loop)))))

(define (evaluate-string-content content uri)
  (define source (or uri 'repl))
  (hash-set! file-content-cache source content)
  (define port (open-input-string content))
  (port-count-lines! port)
  
  (define ns
    (if uri
        (hash-ref! document-namespaces uri (lambda () (make-base-namespace)))
        (current-namespace)))

  (parameterize ([current-namespace ns])
    (let loop ()
      (define pos (file-position port))
      (with-handlers ([exn:fail? (lambda (e)
                                   (define-values (l c end-c) (get-exn-location e #f source))
                                   (display-result (make-range l c l end-c) e #:is-error #t)
                                   (file-position port pos)
                                   (read-line port)
                                   (loop))])
        (define stx (read-syntax source port))
        (unless (eof-object? stx)
          (with-handlers ([exn:fail? (lambda (e)
                                       (define-values (l c end-c) (get-exn-location e stx source))
                                       (display-result (make-range l (or (syntax-column stx) 0) l end-c) e #:is-error #t)
                                       (loop))])
            (define expanded (expand stx))
            (syntax-case expanded (module)
              [(module name lang (mb . body))
               (let ([m-name (syntax-e #'name)])
                 (eval expanded)
                 (dynamic-require `(quote ,m-name) #f)
                 (define ns (module->namespace `(quote ,m-name)))
                 (for ([form (syntax->list #'body)])
                   (evaluate-single-form form ns source)))]
              [_
               (evaluate-single-form stx (current-namespace) source)])
            (loop)))))))



(define file-content-cache (make-hash))
(define (get-file-content path)
  (if (symbol? path)
      "" ;; No content for REPL symbols
      (hash-ref! file-content-cache path
                 (lambda () (file->string path)))))

(define (get-exn-location e stx target-path)
  (let ([loc (and (exn:srclocs? e) 
                  (pair? ((exn:srclocs-accessor e) e))
                  (car ((exn:srclocs-accessor e) e)))])
    (if (and loc (srcloc-line loc) (srcloc-column loc))
        (values (srcloc-line loc) (srcloc-column loc) (+ (srcloc-column loc) (or (srcloc-span loc) 0)))
        (let-values ([(l c) (get-syntax-end stx target-path)])
          (values l (or (syntax-column stx) 0) c)))))

(define (get-syntax-end stx target-path)
  (let ([pos (and stx (syntax-position stx))]
        [span (and stx (syntax-span stx))]
        [line (and stx (syntax-line stx))]
        [col (and stx (syntax-column stx))])
    (if (and stx (not (symbol? target-path)) pos span line col)
        (let* ([content (get-file-content target-path)]
               ;; Substring can be out of bounds if file changed, so guard it
               [sub (if (<= (+ pos -1 span) (string-length content))
                        (substring content (- pos 1) (+ pos -1 span))
                        "")]
               [lines (string-split sub "\n" #:trim? #f)])
          (if (<= (length lines) 1)
              (values line (+ col span))
              (let ([last-line (last lines)])
                (values (+ line (length lines) -1)
                        (string-length last-line)))))
        (values (or line 1) (+ (or col 0) (or span 0))))))

(define (make-range line col end-line end-col)
  (hasheq 'line (or line 1)
          'col (or col 0)
          'end_line (or end-line line 1)
          'end_col (or end-col col 999)))

(define (display-result range val #:is-error [is-error #f] #:output [output ""])
  (define base
    (hash-set* range
               'result (if (exn? val) (exn-message val) (format "~v" val))
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
