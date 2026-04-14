use std::collections::HashMap;
use lsp_types::{TextDocumentItem, TextDocumentContentChangeEvent};

#[derive(Debug, Clone)]
pub struct Document {
    pub _uri: String,
    pub _language_id: String,
    pub version: i32,
    pub text: String,
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
        self.documents.insert(
            item.uri.to_string(),
            Document {
                _uri: item.uri.to_string(),
                _language_id: item.language_id,
                version: item.version,
                text: item.text,
            },
        );
    }

    pub fn change(&mut self, uri: &str, version: i32, changes: Vec<TextDocumentContentChangeEvent>) {
        if let Some(doc) = self.documents.get_mut(uri) {
            doc.version = version;
            // Since we currently use FULL sync, the last change contains the full text.
            if let Some(change) = changes.into_iter().last() {
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
