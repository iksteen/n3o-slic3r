//! 3MF container helpers.
//!
//! A 3MF file is just a zip archive with a fixed entry layout
//! ([Content_Types].xml, `_rels/.rels`, `3D/3dmodel.model`, optional
//! `3D/Objects/object_N.model` side files, and metadata under
//! `Metadata/*`). This module wraps `zip::ZipArchive` with the small
//! conveniences the loader needs: case-insensitive entry lookup
//! (Bambu Studio writes some paths with a leading slash and mixed
//! case across versions), `read_opt` for entries that may legitimately
//! be absent, and existence probes.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use crate::core::scene::loaders::LoadError;

pub struct Container {
    path: PathBuf,
    archive: zip::ZipArchive<BufReader<File>>,
    /// Lowercased entry name → canonical entry name from the zip.
    /// Resolves the leading-slash + case-mismatch wrinkle once,
    /// up front, instead of re-doing the work on every lookup.
    name_index: std::collections::HashMap<String, String>,
}

impl Container {
    pub fn open(path: &Path) -> Result<Self, LoadError> {
        let file = File::open(path).map_err(|e| LoadError::Io {
            path: path.into(),
            source: e,
        })?;
        let archive = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| LoadError::Parse {
            path: path.into(),
            message: format!("not a valid 3MF (zip): {e}"),
        })?;

        let mut name_index = std::collections::HashMap::new();
        for name in archive.file_names() {
            name_index.insert(canonicalize(name), name.to_owned());
        }

        Ok(Self {
            path: path.into(),
            archive,
            name_index,
        })
    }

    /// Read a required entry. The lookup key is canonicalized (lower-
    /// cased + leading-slash stripped) so callers can pass either
    /// `"3D/3dmodel.model"` or `"/3D/3dmodel.model"`.
    pub fn read(&mut self, entry: &str) -> Result<Vec<u8>, LoadError> {
        match self.read_opt(entry)? {
            Some(bytes) => Ok(bytes),
            None => Err(LoadError::Parse {
                path: self.path.clone(),
                message: format!("missing required 3MF entry: {entry}"),
            }),
        }
    }

    /// Read an optional entry. Returns `Ok(None)` if the entry is
    /// absent, `Err` only on read/decompress errors.
    pub fn read_opt(&mut self, entry: &str) -> Result<Option<Vec<u8>>, LoadError> {
        let key = canonicalize(entry);
        let Some(canonical) = self.name_index.get(&key).cloned() else {
            return Ok(None);
        };
        let mut file = self
            .archive
            .by_name(&canonical)
            .map_err(|e| LoadError::Parse {
                path: self.path.clone(),
                message: format!("3MF entry {canonical}: {e}"),
            })?;
        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf).map_err(|e| LoadError::Io {
            path: self.path.clone(),
            source: e,
        })?;
        Ok(Some(buf))
    }
}

fn canonicalize(name: &str) -> String {
    name.trim_start_matches('/').to_ascii_lowercase()
}
