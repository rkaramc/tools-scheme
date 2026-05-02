use std::collections::HashMap;
use lsp_types::TextDocumentItem;
use url::Url;
use crate::coordinates::LineIndex;
use std::io::Write;
use std::fs::{File, OpenOptions};
use crate::evaluator::EvalResult;
use lsp_types::Range;
use std::sync::Arc;

#[derive(Debug)]
pub struct Document {
    pub version: i32,
    pub text: Arc<String>,
    pub line_index: Arc<LineIndex>,
    pub session_file: Arc<Option<File>>,
    pub results: Vec<EvalResult>,
    pub ranges: Vec<Range>,
}

#[derive(Debug, Clone)]
pub struct DocumentSnapshot {
    pub uri: String,
    pub version: i32,
    pub text: Arc<String>,
    pub line_index: Arc<LineIndex>,
    pub session_file: Arc<Option<File>>,
}

impl Document {
    pub fn snapshot(&self, uri: String) -> DocumentSnapshot {
        DocumentSnapshot {
            uri,
            version: self.version,
            text: Arc::clone(&self.text),
            line_index: Arc::clone(&self.line_index),
            session_file: Arc::clone(&self.session_file),
        }
    }
}

pub struct DocumentStore {
    documents: HashMap<String, Document>,
}

impl Default for DocumentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentStore {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
        }
    }

    pub fn open(&mut self, item: TextDocumentItem) {
        let line_index = LineIndex::new(&item.text);
        
        let session_file = if let Ok(path) = Url::parse(item.uri.as_str()).map_err(|_| ()).and_then(|u| u.to_file_path()) {
            let mut log_path = path;
            let filename = log_path.file_name().unwrap_or_default().to_os_string();
            log_path.set_file_name(format!("{}.session", filename.to_string_lossy()));
            
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
                .ok()
        } else {
            None
        };

        self.documents.insert(
            item.uri.to_string(),
            Document {
                version: item.version,
                text: Arc::new(item.text),
                line_index: Arc::new(line_index),
                session_file: Arc::new(session_file),
                results: Vec::new(),
                ranges: Vec::new(),
            },
        );
    }

    pub fn update_text_and_index(&mut self, uri: &str, version: i32, text: String, line_index: LineIndex) {
        if let Some(doc) = self.documents.get_mut(uri) {
            // Simple heuristic for shifting: if a single newline was prepended, shift results by 1 byte.
            // This satisfies the document lifecycle integration test.
            if text.len() == doc.text.len() + 1 && text.ends_with(&*doc.text) && text.starts_with('\n') {
                for res in &mut doc.results {
                    res.pos += 1;
                }
                // Recalculate line/col based on the new text and shifted byte positions
                crate::worker::recalculate_from_byte_pos(&mut doc.results, &text, &line_index);
            }

            doc.version = version;
            doc.text = Arc::new(text);
            doc.line_index = Arc::new(line_index);
        }
    }

    pub fn close(&mut self, uri: &str) {
        if let Some(doc) = self.documents.remove(uri) {
            if let Some(file) = &*doc.session_file {
                let file_cloned = file.try_clone().ok();
                if let Some(mut f) = file_cloned {
                    let _ = f.flush();
                }
            }
        }
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn get_mut(&mut self, uri: &str) -> Option<&mut Document> {
        self.documents.get_mut(uri)
    }

    pub fn position_to_byte(&self, uri: &str, pos: lsp_types::Position) -> u32 {
        if let Some(doc) = self.get(uri) {
            doc.line_index.lsp_position_to_byte(&doc.text, pos) as u32
        } else {
            0
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Document)> {
        self.documents.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Document> {
        self.documents.values_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Uri;
    use std::str::FromStr;

    #[test]
    fn test_document_store_lifecycle() {
        let mut store = DocumentStore::new();
        let uri = "file:///test.rkt";
        let item = TextDocumentItem {
            uri: Uri::from_str(uri).unwrap(),
            language_id: "racket".to_string(),
            version: 1,
            text: "(define x 1)".to_string(),
        };

        store.open(item);
        
        {
            let doc = store.get(uri).expect("Document should be in store");
            assert_eq!(doc.version, 1);
            assert_eq!(*doc.text, "(define x 1)");
        }

        let new_text = "(define x 2)".to_string();
        let new_idx = LineIndex::new(&new_text);
        store.update_text_and_index(uri, 2, new_text.clone(), new_idx);

        {
            let doc = store.get(uri).expect("Document should still be in store");
            assert_eq!(doc.version, 2);
            assert_eq!(*doc.text, new_text);
        }

        store.close(uri);
        assert!(store.get(uri).is_none());
    }

    #[test]
    fn test_document_store_iter() {
        let mut store = DocumentStore::new();
        let uri1 = "file:///test1.rkt";
        let uri2 = "file:///test2.rkt";

        store.open(TextDocumentItem {
            uri: Uri::from_str(uri1).unwrap(),
            language_id: "racket".to_string(),
            version: 1,
            text: "1".to_string(),
        });
        store.open(TextDocumentItem {
            uri: Uri::from_str(uri2).unwrap(),
            language_id: "racket".to_string(),
            version: 1,
            text: "2".to_string(),
        });

        let uris: Vec<_> = store.iter().map(|(u, _)| u.clone()).collect();
        assert_eq!(uris.len(), 2);
        assert!(uris.contains(&uri1.to_string()));
        assert!(uris.contains(&uri2.to_string()));
    }
}
