#lang racket

(require json racket/exn)

(define cache-counter 0)
(define MAX-CACHE-SIZE 100)
(define MAX-OUTPUT-SIZE 10000)

(define real-stdout (current-output-port))
(define file-content-cache (make-hash))
(define cache-access-log (make-hash))
(define document-namespaces (make-hash))

(define (cache-content! content source)
  (set! cache-counter (+ cache-counter 1))
  (hash-set! cache-access-log source cache-counter)

  (hash-set! file-content-cache source content)

  ;; Evict if too large
  (when (> (hash-count file-content-cache) MAX-CACHE-SIZE)
    (define-values (oldest-source _)
      (for/fold ([min-s #f] [min-v +inf.0])
                ([(s v) (in-hash cache-access-log)])
        (if (< v min-v) (values s v) (values min-s min-v))))
    (when oldest-source
      (hash-remove! file-content-cache oldest-source)
      (hash-remove! cache-access-log oldest-source)))

  content)

(define (get-cached-content path)
  (if (or (not path) (symbol? path))
      ""
      (begin
        (set! cache-counter (+ cache-counter 1))
        (hash-set! cache-access-log path cache-counter)
        (hash-ref! file-content-cache path
                   (lambda ()
                     (file->string path))))))

(define (make-range line col end-line end-col span pos)
  (hasheq 'line (or line 1)
          'col (or col 0)
          'end_line (or end-line line 1)
          'end_col (or end-col col 999)
          'span (or span 0)
          'pos (or pos 1)))

(define (truncate-string str limit)
  (if (> (string-length str) limit)
      (string-append (substring str 0 limit) "... [truncated]")
      str))

(define (display-result range val #:is-error [is-error #f] #:output [output ""])
  (define result-str
    (truncate-string (if (exn? val) (exn-message val) (format "~v" val)) MAX-OUTPUT-SIZE))
  (define output-str
    (truncate-string output MAX-OUTPUT-SIZE))

  (define base
    (hash-set* range
               'result result-str
               'is_error is-error
               'output output-str))
  (displayln (jsexpr->string base) real-stdout)
  (flush-output real-stdout))

(define (get-syntax-end stx)
  (let ([pos (and stx (syntax-position stx))]
        [span (and stx (syntax-span stx))]
        [line (and stx (syntax-line stx))]
        [col (and stx (syntax-column stx))])
    (values (or line 1) (+ (or col 0) (or span 0)))))

(define (get-exn-location e stx)
  (let ([loc (and (exn:srclocs? e)
                  (pair? ((exn:srclocs-accessor e) e))
                  (car ((exn:srclocs-accessor e) e)))])
    (if (and loc (srcloc-line loc) (srcloc-column loc))
        (values (srcloc-line loc) (srcloc-column loc) (+ (srcloc-column loc) (or (srcloc-span loc) 0)) (or (srcloc-span loc) 0) (srcloc-position loc))
        (let-values ([(l c) (get-syntax-end stx)])
          (values l (or (and stx (syntax-column stx)) 0) c (or (and stx (syntax-span stx)) 0) (or (and stx (syntax-position stx)) 1))))))

;; --- Core Evaluation Engine ---

(define (for-each-syntax port source proc)
  (let loop ()
    (define pos (file-position port))
    (with-handlers ([exn:fail? (lambda (e)
                                 (define-values (l c end-c span byte-pos) (get-exn-location e #f))
                                 (display-result (make-range l c l end-c span byte-pos) e #:is-error #t)
                                 (file-position port pos)
                                 (read-line port)
                                 (loop))])
      (define stx (read-syntax source port))
      (unless (eof-object? stx)
        (proc stx)
        (loop)))))

(define (evaluate-single-form stx ns)
  (with-handlers ([exn:fail? (lambda (e)
                               (define-values (l c end-c span pos) (get-exn-location e stx))
                               (display-result (make-range l (or (syntax-column stx) 0) l end-c span pos) e #:is-error #t))])
    (define capture-port (open-output-string))
    (define result
      (parameterize ([current-output-port capture-port]
                     [current-namespace ns])
        (eval stx)))
    (define captured (get-output-string capture-port))
    (define-values (end-line end-col) (get-syntax-end stx))
    (define start-line (or (syntax-line stx) 1))
    (define start-col (or (syntax-column stx) 0))
    (define span (or (syntax-span stx) 0))
    (define pos (or (syntax-position stx) 1))

    (cond
      [(not (void? result))
       (display-result (make-range start-line start-col end-line end-col span pos) result #:output captured)]
      [(not (string=? captured ""))
       (display-result (make-range start-line start-col end-line end-col span pos) 'void #:output captured)])))

(define (evaluate-port port source ns)
  (parameterize ([current-namespace ns])
    (for-each-syntax port source
                     (lambda (stx)
                       (with-handlers ([exn:fail? (lambda (e)
                                                    (define-values (l c end-c span pos) (get-exn-location e stx))
                                                    (display-result (make-range l (or (syntax-column stx) 0) l end-c span pos) e #:is-error #t))])
                         (define expanded (expand stx))
                         (syntax-case expanded (module)
                           [(module name lang (mb . body))
                            (let ([m-ns (current-namespace)])
                              (with-handlers ([exn:fail? (lambda (e)
                                                           ;; If language requirement fails, try body anyway
                                                           (for ([form (syntax->list #'body)])
                                                             (evaluate-single-form form m-ns)))])
                                (namespace-require (syntax->datum #'lang))
                                (for ([form (syntax->list #'body)])
                                  (evaluate-single-form form m-ns))))]
                           [_
                            (evaluate-single-form stx (current-namespace))]))))))

;; --- Entry Points ---

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
                 [read-accept-lang #t])
    (evaluate-port port target-path (make-base-namespace)))

  (when (string=? path "-") (delete-file target-path)))

(define (parse-string-content content uri)
  (define source (or uri 'parser))
  (define cached (cache-content! content source))
  (define port (open-input-string cached))
  (port-count-lines! port)
  (for-each-syntax port source
                   (lambda (stx)
                     (define-values (end-line end-col) (get-syntax-end stx))
                     (define start-line (or (syntax-line stx) 1))
                     (define start-col (or (syntax-column stx) 0))
                     (define span (or (syntax-span stx) 0))
                     (define pos (or (syntax-position stx) 1))
                     (define range (hash-set (make-range start-line start-col end-line end-col span pos) 'type "range"))
                     (displayln (jsexpr->string range) real-stdout))))

(define (evaluate-string-content content uri)
  (define source (or uri 'repl))
  (define cached (cache-content! content source))
  (define port (open-input-string cached))
  (port-count-lines! port)
  (define ns
    (if uri
        (hash-ref! document-namespaces uri (lambda () (make-base-namespace)))
        (current-namespace)))
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (evaluate-port port source ns)))

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
              [(string=? type "evaluate") (evaluate-string-content (hash-ref json-input 'content) uri)]
              [(string=? type "parse") (parse-string-content (hash-ref json-input 'content) uri)]
              [(string=? type "clear-namespace") (when uri (hash-remove! document-namespaces uri))]
              [else (eprintf "Unknown REPL command type: ~a\n" type)]))
          (displayln "READY" real-stdout)
          (flush-output real-stdout)
          (loop))))))

(module+ main
  (require racket/cmdline)
  (command-line
    #:program "eval-shim"
    #:once-each
    [("--repl") "Run in persistent REPL mode" (run-repl) (exit 0)]
    #:args (filename)
    (evaluate-file filename)))
