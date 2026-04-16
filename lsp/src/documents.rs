use std::collections::HashMap;
use lsp_types::{TextDocumentItem, TextDocumentContentChangeEvent};
use crate::coordinates::LineIndex;
use std::io::Write;
use std::fs::{File, OpenOptions};

#[derive(Debug)]
pub struct Document {
    pub version: i32,
    pub text: String,
    pub line_index: LineIndex,
    pub session_file: Option<File>,
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
        
        let session_file = if let Ok(path) = item.uri.to_file_path() {
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
                text: item.text,
                line_index,
                session_file,
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
        if let Some(mut doc) = self.documents.remove(uri) {
            if let Some(ref mut file) = doc.session_file {
                let _ = file.flush();
            }
        }
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }
}
