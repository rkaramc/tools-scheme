#lang racket

(require json racket/exn racket/sandbox racket/snip racket/class racket/draw net/base64)

(define cache-counter 0)
(define MAX-CACHE-SIZE (make-parameter 100))
(define MAX-OUTPUT-SIZE (make-parameter 10000))

(define current-repl-output-port (make-parameter (current-output-port)))
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
  (when (> (hash-count file-content-cache) (MAX-CACHE-SIZE))
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
      [else (truncate-string (if (exn? val) (exn-message val) (format "~v" val)) (MAX-OUTPUT-SIZE))]))
  
  (define output-str
    (truncate-string output (MAX-OUTPUT-SIZE)))

  (define base
    (hash-set* range
               'result result-str
               'is_error is-error
               'output output-str))
  
  ;; For rich media, also emit a dedicated rich payload if it's a snip
  (when snip
    (define rich (hash 'type "rich" 'mime "image/png" 'data (snip->base64-png val)))
    (displayln (jsexpr->string rich) (current-repl-output-port)))

  (displayln (jsexpr->string base) (current-repl-output-port))
  (flush-output (current-repl-output-port)))

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

(define (block-evaluable? text)
  (with-handlers ([exn:fail? (lambda (e) #f)])
    (define port (open-input-string text))
    (not (eof-object? (read-syntax 'validator port)))))

(define (count-line-col text start-line start-col)
  (let loop ([i 0] [l start-line] [c start-col])
    (if (= i (string-length text))
        (values l c)
        (let ([ch (string-ref text i)])
          (cond
            [(char=? ch #\newline) (loop (+ i 1) (+ l 1) 0)]
            [(char=? ch #\return)
             (if (and (< (+ i 1) (string-length text))
                      (char=? (string-ref text (+ i 1)) #\newline))
                 (loop (+ i 2) (+ l 1) 0)
                 (loop (+ i 1) (+ l 1) 0))]
            [else (loop (+ i 1) l (+ c 1))])))))

(define (emit-block-range start-line start-col end-line end-col span pos kind valid?)
  (define range (hash 'type "range"
                      'line start-line
                      'col start-col
                      'end_line end-line
                      'end_col end-col
                      'span span
                      'pos pos
                      'kind kind
                      'valid valid?))
  (displayln (jsexpr->string range) (current-repl-output-port)))

(define (parse-string-content content uri)
  (define split-re #px"(?m:(\r?\n[ \t]*){2,})")
  (define md-start-re #px"#\\|\\s*markdown\\s*")
  
  (define (process-code-blocks text start-pos start-line start-col)
    (let loop ([start 0] [current-pos start-pos] [current-line start-line] [current-col start-col])
      (define m (regexp-match-positions split-re text start))
      (define block-end (if m (caar m) (string-length text)))
      (define block-text (substring text start block-end))
      (define span (bytes-length (string->bytes/utf-8 block-text)))
      
      (define-values (after-block-line after-block-col)
        (count-line-col block-text current-line current-col))
      
      (when (> span 0)
        (define is-valid (block-evaluable? block-text))
        (emit-block-range current-line current-col after-block-line after-block-col span current-pos "code" is-valid))
      
      (when m
        (let* ([sep-start (caar m)]
               [sep-end (cdar m)]
               [sep-text (substring text sep-start sep-end)]
               [sep-span (bytes-length (string->bytes/utf-8 sep-text))])
          (let-values ([(next-line next-col) (count-line-col sep-text after-block-line after-block-col)])
            (loop sep-end (+ current-pos span sep-span) next-line next-col))))))

  (let loop ([start 0] [current-pos 1] [current-line 1] [current-col 0])
    (define m (regexp-match-positions md-start-re content start))
    (cond
      [m
       (let* ([pre-md-end (caar m)]
              [pre-md-text (substring content start pre-md-end)])
         ;; Process code before the markdown block
         (process-code-blocks pre-md-text current-pos current-line current-col)
         
         (let*-values ([(md-start-line md-start-col) (count-line-col pre-md-text current-line current-col)]
                       [(md-start-pos) (+ current-pos (bytes-length (string->bytes/utf-8 pre-md-text)))]
                       [(md-content-start) (cdar m)]
                       [(md-end-match) (regexp-match-positions #px"\\|#" content md-content-start)])
           (cond
             [md-end-match
              (let* ([md-end-pos (caar md-end-match)]
                     [full-md-text (substring content (caar m) (+ md-end-pos 2))]
                     [full-md-span (bytes-length (string->bytes/utf-8 full-md-text))])
                (let-values ([(md-end-line md-end-col) (count-line-col full-md-text md-start-line md-start-col)])
                  (emit-block-range md-start-line md-start-col md-end-line md-end-col full-md-span md-start-pos "markdown" #t)
                  (loop (+ md-end-pos 2) (+ md-start-pos full-md-span) md-end-line md-end-col)))]
             [else
              ;; Unclosed markdown block - treat rest of file as markdown
              (let* ([full-md-text (substring content (caar m))]
                     [full-md-span (bytes-length (string->bytes/utf-8 full-md-text))])
                (let-values ([(md-end-line md-end-col) (count-line-col full-md-text md-start-line md-start-col)])
                  (emit-block-range md-start-line md-start-col md-end-line md-end-col full-md-span md-start-pos "markdown" #t)))])))]
      [else
       ;; Process remaining code
       (process-code-blocks (substring content start) current-pos current-line current-col)])))


(define (uri-decode str)
  ;; Minimal percent-decoder for path characters (handles %3A → ":" etc.).
  (define (hex-digit? c) (or (char-alphabetic? c) (char-numeric? c)))
  (define bytes (string->bytes/latin-1 str))
  (define out (open-output-bytes))
  (let loop ([i 0])
    (when (< i (bytes-length bytes))
      (define b (bytes-ref bytes i))
      (if (and (= b 37)                        ; #\%
               (< (+ i 2) (bytes-length bytes)))
          (let ([h1 (integer->char (bytes-ref bytes (+ i 1)))]
                [h2 (integer->char (bytes-ref bytes (+ i 2)))])
            (if (and (hex-digit? h1) (hex-digit? h2))
                (let ([num (string->number (string h1 h2) 16)])
                  (if num
                      (begin (write-byte num out) (loop (+ i 3)))
                      (begin (write-byte b out) (loop (+ i 1)))))
                (begin (write-byte b out) (loop (+ i 1)))))
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
     (displayln (jsexpr->string msg) (current-repl-output-port))
     (flush-output (current-repl-output-port))
     (- end start))
   void))

(define (get-evaluator uri content)
  (hash-ref! document-evaluators uri
             (lambda ()
                (define file-path (uri->path uri))
                ;; Only try to get directory if we have a valid path (ts-k2w)
                (define file-dir (and file-path 
                                      (let ([p (path->complete-path file-path)])
                                        (if (directory-exists? p) p (path-only p)))))
                (define port (open-input-string content))
                (port-count-lines! port)
               
               ;; read-language parses the #lang header but does NOT consume 
               ;; the module body (unlike read-syntax).
               (define lang-spec (with-handlers ([exn:fail? (lambda (e) 'racket/base)])
                                   (parameterize ([read-accept-reader #t]
                                                  [read-accept-lang #t])
                                     (read-language port (lambda () 'racket/base)))))
               
               ;; If read-language returns a procedure (the 'get-info' handler),
               ;; make-evaluator often rejects it as a 'bad language spec'.
               ;; We normalize it to a known safe module path symbol.
               (define lang
                 (if (procedure? lang-spec)
                     'racket
                     lang-spec))

               ;; Create evaluator with reasonable student limits
               (parameterize ([sandbox-namespace-specs
                               (list make-base-namespace
                                     'racket/class
                                     'racket/snip)]
                              [sandbox-output (make-streaming-port 'stdout uri)]
                              [sandbox-error-output (make-streaming-port 'stderr uri)]
                              [sandbox-memory-limit 128] ; 128MB
                              [sandbox-eval-limits '(15 128)] ; 15s, 128MB
                              ;; Set directory parameters so the sandbox starts in the right place (ts-k2w)
                              [current-directory (or file-dir (current-directory))]
                              [current-load-relative-directory file-dir])
                 (if file-dir
                     (make-evaluator lang #:allow-read (list file-dir))
                     (make-evaluator lang))))))

(define (evaluate-string-content content uri)
  (define ev (get-evaluator (or uri "repl") content))
  (define file-path (uri->path uri))
  (define source    (or file-path uri 'repl))
  (define cached (cache-content! content (or uri 'repl)))
  (define port (open-input-string cached))
  (port-count-lines! port)
  
  ;; Consume the #lang header if present, so for-each-syntax starts at the 
  ;; first real expression. This ensures we evaluate forms individually 
  ;; even in #lang files, which is required for ts-h31.
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (define pos (file-position port))
    (unless (with-handlers ([exn:fail? (lambda (e) #f)])
              (read-language port (lambda () #f)))
      (file-position port pos)))

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

(define (validate-blocks blocks)
  (for ([block (in-list blocks)]
        [i (in-naturals)])
    (define msg (hash 'type "validation" 'index i 'valid (block-evaluable? block)))
    (displayln (jsexpr->string msg) (current-repl-output-port))
    (flush-output (current-repl-output-port))))

(define (run-repl)
  (parameterize ([read-accept-reader #t]
                 [read-accept-lang #t])
    (let loop ()
      (define input (read-line))
      (unless (eof-object? input)
        (with-handlers ([exn:fail? (lambda (e)
                                     (display-result (make-range 1 0 1 0 0 1) e #:is-error #t)
                                     (displayln "READY" (current-repl-output-port))
                                     (flush-output (current-repl-output-port))
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
                                                      (displayln "READY" (current-repl-output-port))
                                                      (flush-output (current-repl-output-port)))]
                                        [exn:fail? (lambda (e)
                                                     ;; Fallback just in case, but evaluate-string-content already catches most exn:fail?
                                                     (displayln "READY" (current-repl-output-port))
                                                     (flush-output (current-repl-output-port)))])
                          (evaluate-string-content (hash-ref json-input 'content) uri)
                          (displayln "READY" (current-repl-output-port))
                          (flush-output (current-repl-output-port))))))]
              [(string=? type "cancel-evaluation")
               (when current-evaluator
                 (break-evaluator current-evaluator))]
              [(string=? type "parse")
               (parse-string-content (hash-ref json-input 'content) uri)
               (displayln "READY" (current-repl-output-port))
               (flush-output (current-repl-output-port))]
              [(string=? type "validate-blocks")
               (validate-blocks (hash-ref json-input 'blocks))
               (displayln "READY" (current-repl-output-port))
               (flush-output (current-repl-output-port))]
              [(string=? type "clear-namespace")
               (when uri (hash-remove! document-evaluators uri))
               (displayln "READY" (current-repl-output-port))
               (flush-output (current-repl-output-port))]
              [else
               (eprintf "Unknown REPL command type: ~a\n" type)
               (displayln "READY" (current-repl-output-port))
               (flush-output (current-repl-output-port))])))
        (loop)))))

(define (reset-cache!)
  (hash-clear! file-content-cache)
  (hash-clear! cache-access-log)
  (set! cache-counter 0))

(module+ main
  (require racket/cmdline)
  (command-line
    #:program "eval-shim"
    #:once-each
    [("--repl") "Run in persistent REPL mode" (run-repl) (exit 0)]
    #:args (filename)
    (evaluate-file filename)))

(module+ test
  (require rackunit)
  
  (test-case "truncate-string"
    (check-equal? (truncate-string "hello" 10) "hello")
    (check-equal? (truncate-string "hello world" 5) "hello... [truncated]"))

  (test-case "make-range"
    (let ([r (make-range 1 2 3 4 5 6)])
      (check-equal? (hash-ref r 'line) 1)
      (check-equal? (hash-ref r 'col) 2)
      (check-equal? (hash-ref r 'end_line) 3)
      (check-equal? (hash-ref r 'end_col) 4)
      (check-equal? (hash-ref r 'span) 5)
      (check-equal? (hash-ref r 'pos) 6))
    (let ([r (make-range #f #f #f #f #f #f)])
      (check-equal? (hash-ref r 'line) 1)
      (check-equal? (hash-ref r 'col) 0)
      (check-equal? (hash-ref r 'end_line) 1)
      (check-equal? (hash-ref r 'end_col) 999)
      (check-equal? (hash-ref r 'span) 0)
      (check-equal? (hash-ref r 'pos) 1)))

  (test-case "uri-decode"
    (check-equal? (uri-decode "hello%20world") "hello world")
    (check-equal? (uri-decode "path%2Fto%2Ffile") "path/to/file")
    (check-equal? (uri-decode "C%3A%5CUsers") "C:\\Users")
    (check-equal? (uri-decode "plain") "plain")
    (check-equal? (uri-decode "trailing%") "trailing%")
    (check-equal? (uri-decode "invalid%2G") "invalid%2G"))

  (test-case "uri->path"
    (check-false (uri->path "not-a-uri"))
    (check-false (uri->path "http://google.com"))
    (if (eq? 'windows (system-type 'os))
        (begin
          (check-equal? (uri->path "file:///C%3A/Users/test.rkt") (string->path "C:\\Users\\test.rkt"))
          (check-equal? (uri->path "file:///D:/source/tools.rkt") (string->path "D:\\source\\tools.rkt")))
        (begin
          (check-equal? (uri->path "file:///home/user/test.rkt") (string->path "/home/user/test.rkt"))
          (check-equal? (uri->path "file:///var/log/sys.log") (string->path "/var/log/sys.log")))))

  (test-case "LRU cache behavior"
    (reset-cache!)
    
    (parameterize ([MAX-CACHE-SIZE 2])
      (cache-content! "content1" "source1")
      (cache-content! "content2" "source2")
      (check-equal? (hash-count file-content-cache) 2)
      
      ;; Source1 is oldest. Adding source3 should evict source1.
      (cache-content! "content3" "source3")
      (check-equal? (hash-count file-content-cache) 2)
      (check-false (hash-has-key? file-content-cache "source1"))
      (check-true (hash-has-key? file-content-cache "source2"))
      (check-true (hash-has-key? file-content-cache "source3"))
      
      ;; Access source2 to make it newer than source3
      (get-cached-content "source2")
      
      ;; Now source3 is oldest. Adding source4 should evict source3.
      (cache-content! "content4" "source4")
      (check-equal? (hash-count file-content-cache) 2)
      (check-false (hash-has-key? file-content-cache "source3"))
      (check-true (hash-has-key? file-content-cache "source2"))
      (check-true (hash-has-key? file-content-cache "source4"))))

  (test-case "evaluate-file"
    (let ([tmp (make-temporary-file "test-eval-file-~a.rkt")])
      (with-output-to-file tmp #:exists 'replace
        (lambda () (displayln "#lang racket\n(define x 10)\n(+ x 5)")))
      (let ([out-port (open-output-string)])
        (parameterize ([current-repl-output-port out-port])
          (evaluate-file (path->string tmp)))
        (check-regexp-match #px"\"result\":\"15\"" (get-output-string out-port)))
      (delete-file tmp)))

  (test-case "REPL unknown command"
    (let* ([in-port (open-input-string "{\"type\": \"unknown\", \"uri\": \"test\"}\n")]
           [out-port (open-output-string)])
      (parameterize ([current-repl-output-port out-port]
                     [current-input-port in-port])
        (run-repl))
      (check-regexp-match #px"READY\n" (get-output-string out-port))))

  (test-case "REPL malformed JSON"
    (let* ([in-port (open-input-string "not json\n")]
           [out-port (open-output-string)])
      (parameterize ([current-repl-output-port out-port]
                     [current-input-port in-port])
        (run-repl))
      (let ([output (get-output-string out-port)])
        (check-regexp-match #px"\"is_error\":true" output)
        (check-regexp-match #px"READY\n" output)))
    ;; Reset error handling for next tests
    (void))

  (test-case "REPL clear-namespace"
    (let* ([in-port (open-input-string "{\"type\": \"clear-namespace\", \"uri\": \"test-uri\"}\n")]
           [out-port (open-output-string)])
      (hash-set! document-evaluators "test-uri" 'some-evaluator)
      (parameterize ([current-repl-output-port out-port]
                     [current-input-port in-port])
        (run-repl))
      (check-regexp-match #px"READY\n" (get-output-string out-port))
      (check-false (hash-has-key? document-evaluators "test-uri"))))

  (test-case "REPL parse blocks"
    (let* ([content "#lang racket\n(+ 1 2)\n\n(+ 3 4)"]
           [in-port (open-input-string (jsexpr->string (hash 'type "parse" 'uri "test-uri" 'content content)))]
           [out-port (open-output-string)])
      (parameterize ([current-repl-output-port out-port]
                     [current-input-port in-port])
        (run-repl))
      (let ([output (get-output-string out-port)])
        (check-regexp-match #px"\"line\":1" output)
        (check-regexp-match #px"\"span\":20" output)
        (check-regexp-match #px"\"line\":4" output)
        (check-regexp-match #px"\"span\":7" output)
        (check-regexp-match #px"READY\n" output))))

  (test-case "REPL parse multi-paragraph markdown"
    (let* ([content "#lang racket\n(define x 10)\n\n#| markdown\nPara 1\n\nPara 2\n|#\n\n(define y 20)\n\n#| markdown Unclosed"]
           [in-port (open-input-string (jsexpr->string (hash 'type "parse" 'uri "test-uri" 'content content)))]
           [out-port (open-output-string)])
      (parameterize ([current-repl-output-port out-port]
                     [current-input-port in-port])
        (run-repl))
      (let ([output (get-output-string out-port)])
        ;; Block 1: Code
        (check-regexp-match #px"\"line\":1" output)
        (check-regexp-match #px"\"kind\":\"code\"" output)
        ;; Block 2: Markdown (with Para 1\n\nPara 2)
        (check-regexp-match #px"\"line\":4" output)
        (check-regexp-match #px"\"kind\":\"markdown\"" output)
        ;; Block 3: Code
        (check-regexp-match #px"\"line\":10" output)
        (check-regexp-match #px"\"kind\":\"code\"" output)
        ;; Block 4: Markdown (Unclosed)
        (check-regexp-match #px"\"line\":12" output)
        (check-regexp-match #px"\"kind\":\"markdown\"" output)
        (check-regexp-match #px"READY\n" output))))

  (test-case "REPL evaluate and cancel"
    ;; We use a thread to run the REPL and pipes to communicate with it
    (let*-values ([(in-rd in-wr) (make-pipe)]
                  [(out-port) (open-output-string)])
      (let ([repl-thread 
             (thread
              (lambda ()
                (parameterize ([current-repl-output-port out-port]
                               [current-input-port in-rd])
                  (run-repl))))])
        
        ;; 1. Test successful evaluation
        (displayln "{\"type\": \"evaluate\", \"uri\": \"test-eval\", \"content\": \"(+ 1 2)\"}" in-wr)
        (flush-output in-wr)
        
        ;; Wait for READY
        (let loop ([retries 20])
          (when (and (not (regexp-match? #px"READY\n" (get-output-string out-port)))
                     (> retries 0))
            (sleep 0.1)
            (loop (- retries 1))))
        
        (let ([output (get-output-string out-port)])
          (check-regexp-match #px"\"result\":\"3\"" output)
          (check-regexp-match #px"READY\n" output))
        
        ;; 2. Test cancellation
        ;; Use a long-running task. We need to be careful with sleep in sandbox.
        ;; Actually, let's use a simple infinite loop or long sleep if allowed.
        (displayln "{\"type\": \"evaluate\", \"uri\": \"test-cancel\", \"content\": \"(sleep 5)\"}" in-wr)
        (flush-output in-wr)
        
        (sleep 0.5) ; Give it time to start
        
        (displayln "{\"type\": \"cancel-evaluation\", \"uri\": \"test-cancel\"}" in-wr)
        (flush-output in-wr)
        
        ;; Wait for cancellation message and READY
        (let loop ([retries 20])
          (when (and (not (regexp-match? #px"Evaluation cancelled" (get-output-string out-port)))
                     (> retries 0))
            (sleep 0.1)
            (loop (- retries 1))))
            
        (let ([output (get-output-string out-port)])
          (check-regexp-match #px"Evaluation cancelled" output)
          (check-regexp-match #px"READY\n" output))
        
        ;; Cleanup
        (close-output-port in-wr)
        (sync/timeout 2 repl-thread)
        (kill-thread repl-thread)))))
