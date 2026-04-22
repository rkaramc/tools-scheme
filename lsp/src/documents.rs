use std::collections::HashMap;
use lsp_types::TextDocumentItem;
use url::Url;
use crate::coordinates::LineIndex;
use std::io::Write;
use std::fs::{File, OpenOptions};
use crate::evaluator::EvalResult;
use lsp_types::Range;

#[derive(Debug)]
pub struct Document {
    pub version: i32,
    pub text: String,
    pub line_index: LineIndex,
    pub session_file: Option<File>,
    pub results: Vec<EvalResult>,
    pub ranges: Vec<Range>,
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
                text: item.text,
                line_index,
                session_file,
                results: Vec::new(),
                ranges: Vec::new(),
            },
        );
    }

    pub fn update_text_and_index(&mut self, uri: &str, version: i32, text: String, line_index: LineIndex) {
        if let Some(doc) = self.documents.get_mut(uri) {
            doc.version = version;
            doc.text = text;
            doc.line_index = line_index;
        }
    }

    pub fn close(&mut self, uri: &str) {
        if let Some(mut file) = self.documents.remove(uri).and_then(|d| d.session_file) {
            let _ = file.flush();
        }
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn get_mut(&mut self, uri: &str) -> Option<&mut Document> {
        self.documents.get_mut(uri)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Document> {
        self.documents.values_mut()
    }
}
