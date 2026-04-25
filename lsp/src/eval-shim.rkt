#lang racket

(require json racket/exn racket/sandbox racket/snip racket/class racket/draw net/base64)

(define cache-counter 0)
(define MAX-CACHE-SIZE 100)
(define MAX-OUTPUT-SIZE 10000)

(define real-stdout (current-output-port))
(define file-content-cache (make-hash))
(define cache-access-log (make-hash))
(define document-evaluators (make-hash))

(define (snip->base64-png s)
  (define-values (w h)
    (let ([wb (box 0)]
          [hb (box 0)]
          [dc (make-object bitmap-dc% (make-bitmap 1 1))])
      (send s get-extent dc 0 0 wb hb #f #f #f #f)
      (values (max 1 (unbox wb)) (max 1 (unbox hb)))))
  (define bmp (make-bitmap (inexact->exact (ceiling w)) (inexact->exact (ceiling h))))
  (define dc (make-object bitmap-dc% bmp))
  (send s draw dc 0 0 0 0 w h 0 0 '())
  (define out (open-output-bytes))
  (send bmp save-file out 'png)
  (bytes->string/utf-8 (base64-encode (get-output-bytes out))))

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
  (define snip (is-a? val snip%))
  (define result-str
    (cond
      [snip "Rich media"]
      [else (truncate-string (if (exn? val) (exn-message val) (format "~v" val)) MAX-OUTPUT-SIZE)]))
  
  (define output-str
    (truncate-string output MAX-OUTPUT-SIZE))

  (define base
    (hash-set* range
               'result result-str
               'is_error is-error
               'output output-str))
  
  ;; For rich media, also emit a dedicated rich payload if it's a snip
  (when snip
    (define rich (hash 'type "rich" 'mime "image/png" 'data (snip->base64-png val)))
    (displayln (jsexpr->string rich) real-stdout))

  (displayln (jsexpr->string base) real-stdout)
  (flush-output real-stdout))

(define (get-syntax-end stx)
  ;; Note: This function provides simplified placeholder coordinates (assuming single-line spans).
  ;; The Language Server Protocol requires end_line and end_col fields in the JSON payload,
  ;; but calculating accurate multi-line boundaries and UTF-16 code unit offsets in Racket 
  ;; is inefficient and complex due to CRLF and emoji handling. 
  ;; Instead, the Rust-side LSP server (in `server.rs` `recalculate_from_byte_pos`) 
  ;; completely ignores these values and recalculates `end_line` and `end_col` precisely 
  ;; using the `span`, `line`, and the raw text buffer.
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

(define (eval-module-body-form form m-ns file-dir)
  ;; Like evaluate-single-form but handles (require "relative.rkt") specially.
  ;;
  ;; In a plain namespace (no enclosing module) relative-string require paths
  ;; are NOT resolved using current-load-relative-directory; they have no
  ;; reference point.  We detect them and resolve to absolute paths using
  ;; file-dir before calling namespace-require. (ts-k2w)
  (define form-list (syntax->list form))
  (if (and file-dir
           form-list
           (>= (length form-list) 2)
           (eq? (syntax-e (car form-list)) 'require))
      ;; Rewrite each relative-string module path to an absolute path.
      (for ([spec (cdr form-list)])
        (define datum (syntax->datum spec))
        (define resolved
          (if (string? datum)
              (build-path file-dir datum)
              datum))
        (with-handlers ([exn:fail? (lambda (e)
                                     (define-values (l c end-c span pos) (get-exn-location e form))
                                     (display-result (make-range l (or (syntax-column form) 0) l end-c span pos) e #:is-error #t))])
          (namespace-require resolved m-ns)))
      ;; Not a require (or no file-dir): evaluate normally.
      (evaluate-single-form form m-ns)))

(define (evaluate-port port source ns)
  ;; Evaluate each top-level form in the port.
  ;;
  ;; For #lang / module forms we detect the structure from the RAW
  ;; (unexpanded) syntax.  After read-syntax, a #lang file produces:
  ;;   (module <name> <lang> (<lang>:module-begin <body-form> ...))
  ;;
  ;; We require the language into the namespace and eval each real body form
  ;; directly, without calling expand on the whole module first.
  ;;
  ;; Why not expand first?
  ;; - ts-h31: expand stamps every identifier with the anonymous
  ;;   '|expanded module| scope.  eval-ing those forms in a separate
  ;;   namespace loses the module instance → "namespace mismatch".
  ;; - ts-k2w: expand resolves (require "...") relative paths during
  ;;   expansion; if current-load-relative-directory is wrong at that
  ;;   moment the file cannot be found.
  ;; Eval-ing raw forms avoids both issues.
  (parameterize ([current-namespace ns])
    (for-each-syntax port source
                     (lambda (stx)
                       (with-handlers ([exn:fail? (lambda (e)
                                                     (define-values (l c end-c span pos) (get-exn-location e stx))
                                                     (display-result (make-range l (or (syntax-column stx) 0) l end-c span pos) e #:is-error #t))])
                         (define stx-list (syntax->list stx))
                         (if (and stx-list
                                  (>= (length stx-list) 4)
                                  (eq? (syntax-e (car stx-list)) 'module))
                             ;; Module form: require lang, eval body forms directly.
                             (let* ([lang       (syntax->datum (caddr stx-list))]
                                    [raw-body   (cdddr stx-list)]
                                    ;; After read-syntax the body is a single
                                    ;; (lang:module-begin form ...) wrapper.
                                    ;; Unwrap it to get the real forms.
                                    [body       (let ([mb (and (= (length raw-body) 1)
                                                               (syntax->list (car raw-body)))])
                                                  (if mb (cdr mb) raw-body))]
                                    [m-ns       (current-namespace)]
                                    ;; File directory for resolving relative requires.
                                    [file-dir   (and (path? source) (path-only source))])
                               (with-handlers ([exn:fail? (lambda (e)
                                                             ;; Language require failed; try body anyway.
                                                             (for ([form body])
                                                               (eval-module-body-form form m-ns file-dir)))])
                                 (namespace-require lang)
                                 (for ([form body])
                                   (eval-module-body-form form m-ns file-dir))))
                             ;; Plain top-level form.
                             (evaluate-single-form stx (current-namespace))))))))

;; --- Entry Points ---

(define (evaluate-file path)
  (define target-path
    (if (string=? path "-")
        (let ([tmp (make-temporary-file "eval-shim-~a.rkt")])
          (with-output-to-file tmp #:exists 'replace
            (lambda () (copy-port (current-input-port) (current-output-port))))
          tmp)
        path))

  (define file-path (if (path? target-path) target-path (string->path target-path)))
  (define file-dir  (path-only (path->complete-path file-path)))

  (define port (open-input-file target-path))
  (port-count-lines! port)
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t]
                 ;; Set load-relative dir so (require "...") resolves
                 ;; against the file's own directory (ts-k2w).
                 [current-load-relative-directory file-dir]
                 [current-directory               file-dir])
    (let ([ns (make-base-namespace)])
      (namespace-attach-module (current-namespace) 'racket/class ns)
      (namespace-attach-module (current-namespace) 'racket/snip ns)
      (evaluate-port port file-path ns)))

  (when (string=? path "-") (delete-file target-path)))

(define (parse-string-content content uri)
  (define source (or uri 'parser))
  (define cached (cache-content! content source))
  (define port (open-input-string cached))
  (port-count-lines! port)
  
  (define (emit-ranges stx)
    (define l (and (syntax? stx) (syntax->list stx)))
    (if (and l (>= (length l) 4) (eq? (syntax-e (car l)) 'module))
        (let* ([body-stx (cadddr l)]
               [body-list (syntax->list body-stx)])
          (if (and body-list (>= (length body-list) 1) (eq? (syntax-e (car body-list)) '#%module-begin))
              (for ([form (cdr body-list)])
                (emit-ranges form))
              (emit-range stx)))
        (emit-range stx)))

  (define (emit-range stx)
    (define-values (end-line end-col) (get-syntax-end stx))
    (define start-line (or (syntax-line stx) 1))
    (define start-col (or (syntax-column stx) 0))
    (define span (or (syntax-span stx) 0))
    (define pos (or (syntax-position stx) 1))
    (define range (hash-set (make-range start-line start-col end-line end-col span pos) 'type "range"))
    (displayln (jsexpr->string range) real-stdout))

  (for-each-syntax port source emit-ranges))

(define (uri-decode str)
  ;; Minimal percent-decoder for path characters (handles %3A → ":" etc.).
  (define (hex-digit? c) (or (char-alphabetic? c) (char-numeric? c)))
  (define bytes (string->bytes/latin-1 str))
  (define out (open-output-bytes))
  (let loop ([i 0])
    (when (< i (bytes-length bytes))
      (define b (bytes-ref bytes i))
      (if (and (= b 37)                        ; #\%
               (< (+ i 2) (bytes-length bytes))
               (hex-digit? (integer->char (bytes-ref bytes (+ i 1))))
               (hex-digit? (integer->char (bytes-ref bytes (+ i 2)))))
          (begin
            (write-byte (string->number
                         (string (integer->char (bytes-ref bytes (+ i 1)))
                                 (integer->char (bytes-ref bytes (+ i 2))))
                         16)
                        out)
            (loop (+ i 3)))
          (begin
            (write-byte b out)
            (loop (+ i 1))))))
  (bytes->string/utf-8 (get-output-bytes out)))

(define (uri->path uri)
  ;; Convert a file:/// URI to a native filesystem path object, or #f.
  ;; On Windows, normalise forward slashes → backslashes so that
  ;; Racket's path operations treat the string as a proper path.
  (and (string? uri)
       (string-prefix? uri "file:///")
       (let* ([raw     (substring uri 8)]
              [decoded (uri-decode raw)]
              [native  (if (eq? 'windows (system-type 'os))
                           (string-replace decoded "/" "\\")
                           decoded)])
         (string->path native))))

(define (make-streaming-port type uri)
  (make-output-port
   (symbol->string type)
   always-evt
   (lambda (s start end non-block? breakable?)
     (define str (bytes->string/utf-8 (subbytes s start end) #\?))
     (define msg (hash 'type "output" 'stream (symbol->string type) 'data str 'uri uri))
     (displayln (jsexpr->string msg) real-stdout)
     (flush-output real-stdout)
     (- end start))
   void))

(define (get-evaluator uri content)
  (hash-ref! document-evaluators uri
             (lambda ()
               (define port (open-input-string content))
               (define lang (with-handlers ([exn:fail? (lambda (e) 'racket/base)])
                              (read-language port (lambda () 'racket/base))))
               ;; Create evaluator with reasonable student limits
               (parameterize ([sandbox-namespace-specs
                               (list make-base-namespace
                                     'racket/class
                                     'racket/snip)]
                              [sandbox-output (make-streaming-port 'stdout uri)]
                              [sandbox-error-output (make-streaming-port 'stderr uri)]
                              [sandbox-memory-limit 128] ; 128MB
                              [sandbox-eval-limits '(15 128)]) ; 15s, 128MB
                 (make-evaluator lang)))))

(define (evaluate-string-content content uri)
  (define ev (get-evaluator (or uri "repl") content))
  (define file-path (uri->path uri))
  (define source    (or file-path uri 'repl))
  (define cached (cache-content! content (or uri 'repl)))
  (define port (open-input-string cached))
  (port-count-lines! port)
  
  (for-each-syntax port source
                   (lambda (stx)
                     (with-handlers ([exn:fail? (lambda (e)
                                                  (define-values (l c end-c span pos) (get-exn-location e stx))
                                                  (display-result (make-range l (or (syntax-column stx) 0) l end-c span pos) e #:is-error #t))])
                       ;; Run expression in sandbox. Output streams directly via streaming-ports.
                       (define result (ev stx))
                       
                       (define-values (end-line end-col) (get-syntax-end stx))
                       (define start-line (or (syntax-line stx) 1))
                       (define start-col (or (syntax-column stx) 0))
                       (define span (or (syntax-span stx) 0))
                       (define pos (or (syntax-position stx) 1))

                       (when (not (void? result))
                         (display-result (make-range start-line start-col end-line end-col span pos) result))))))

(define current-eval-thread #f)
(define current-evaluator #f)

(define (run-repl)
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (let loop ()
      (define input (read-line))
      (unless (eof-object? input)
        (with-handlers ([exn:fail? (lambda (e)
                                     (display-result (make-range 1 0 1 0 0 1) e #:is-error #t)
                                     (displayln "READY" real-stdout)
                                     (flush-output real-stdout)
                                     (loop))])
          (let* ([json-input (string->jsexpr input)]
                 [type (hash-ref json-input 'type)]
                 [uri (hash-ref json-input 'uri #f)])
            (cond
              [(string=? type "evaluate")
               (set! current-evaluator (get-evaluator (or uri "repl") (hash-ref json-input 'content)))
               (set! current-eval-thread
                     (thread
                      (lambda ()
                        (with-handlers ([exn:break? (lambda (e)
                                                      (display-result (make-range 1 0 1 0 0 1) (make-exn:fail "Evaluation cancelled" (current-continuation-marks)) #:is-error #t)
                                                      (displayln "READY" real-stdout)
                                                      (flush-output real-stdout))]
                                        [exn:fail? (lambda (e)
                                                     ;; Fallback just in case, but evaluate-string-content already catches most exn:fail?
                                                     (displayln "READY" real-stdout)
                                                     (flush-output real-stdout))])
                          (evaluate-string-content (hash-ref json-input 'content) uri)
                          (displayln "READY" real-stdout)
                          (flush-output real-stdout)))))]
              [(string=? type "cancel-evaluation")
               (when current-evaluator
                 (break-evaluator current-evaluator))]
              [(string=? type "parse")
               (parse-string-content (hash-ref json-input 'content) uri)
               (displayln "READY" real-stdout)
               (flush-output real-stdout)]
              [(string=? type "clear-namespace")
               (when uri (hash-remove! document-evaluators uri))
               (displayln "READY" real-stdout)
               (flush-output real-stdout)]
              [else
               (eprintf "Unknown REPL command type: ~a\n" type)
               (displayln "READY" real-stdout)
               (flush-output real-stdout)])))
        (loop)))))

(module+ main
  (require racket/cmdline)
  (command-line
    #:program "eval-shim"
    #:once-each
    [("--repl") "Run in persistent REPL mode" (run-repl) (exit 0)]
    #:args (filename)
    (evaluate-file filename)))
