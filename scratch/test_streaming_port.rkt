#lang racket
(require racket/sandbox json)

(define real-stdout (current-output-port))

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

(define ev (parameterize ([sandbox-output (make-streaming-port 'stdout "file:///test.rkt")]
                          [sandbox-error-output (make-streaming-port 'stderr "file:///test.rkt")])
             (make-evaluator 'racket/base)))

(ev '(display "Hello World\n"))
(ev '(eprintf "Error output\n"))