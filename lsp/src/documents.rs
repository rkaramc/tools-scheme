use std::collections::HashMap;
use lsp_types::{TextDocumentItem, TextDocumentContentChangeEvent};
use crate::coordinates::LineIndex;

#[derive(Debug, Clone)]
pub struct Document {
    pub version: i32,
    pub text: String,
    pub line_index: LineIndex,
}

pub struct DocumentStore {
    documents: HashMap<String, Document>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
        }
    }

    pub fn open(&mut self, item: TextDocumentItem) {
        let line_index = LineIndex::new(&item.text);
        self.documents.insert(
            item.uri.to_string(),
            Document {
                version: item.version,
                text: item.text,
                line_index,
            },
        );
    }

    pub fn change(&mut self, uri: &str, version: i32, changes: Vec<TextDocumentContentChangeEvent>) {
        if let Some(doc) = self.documents.get_mut(uri) {
            doc.version = version;
            // Since we currently use FULL sync, the last change contains the full text.
            if let Some(change) = changes.into_iter().last() {
                doc.line_index = LineIndex::new(&change.text);
                doc.text = change.text;
            }
        }
    }

    pub fn close(&mut self, uri: &str) {
        self.documents.remove(uri);
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }
}
