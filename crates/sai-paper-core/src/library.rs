//! Local PDF discovery, hashing, and filesystem watching.

use std::{
    fs::File,
    io::{self},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use thiserror::Error;
use walkdir::WalkDir;


/// Metadata collected from a local PDF before database ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPdf {
    /// Absolute or configured source path.
    pub path: PathBuf,
    /// Human-readable title inferred from the filename.
    pub title: String,
    /// SHA-256 content digest used for duplicate detection.
    pub content_hash: String,
    /// File size in bytes.
    pub file_size: u64,
    /// Configured library root containing this file.
    pub library_root: Option<PathBuf>,
    /// Directory relative to the library root, excluding the filename.
    pub relative_directory: Option<PathBuf>,
}

/// A subdirectory discovered beneath a configured library root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionDirectory {
    /// Configured library root containing the directory.
    pub library_root: PathBuf,
    /// Directory path relative to the library root.
    pub relative_path: PathBuf,
}

/// Local library indexing errors.
#[derive(Debug, Error)]
pub enum LibraryError {
    /// Reading a PDF failed.
    #[error("could not read PDF: {0}")]
    Io(#[from] io::Error),
}

/// Stateless recursive PDF indexer.
#[derive(Debug, Default, Clone, Copy)]
pub struct LibraryIndexer;

impl LibraryIndexer {
    /// Recursively scan configured roots and collect readable PDF metadata.
    #[must_use]
    pub fn scan(roots: &[PathBuf]) -> Vec<ImportedPdf> {
        let mut seen = std::collections::HashSet::new();
        roots
            .iter()
            .flat_map(|root| WalkDir::new(root).follow_links(false).into_iter())
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_pdf(entry.path()))
            // 翻訳成果物（<stem>-dual.pdf / <stem>-ja.pdf）は元論文の一部であり、
            // 独立した論文として扱われるべきではないためスキャンから除外する
            .filter(|entry| !is_translated_pdf(entry.path()))
            .filter(|entry| {
                let p = std::fs::canonicalize(entry.path())
                    .unwrap_or_else(|_| entry.path().to_path_buf());
                seen.insert(p)
            })
            .filter_map(|entry| Self::inspect_in_roots(entry.path(), roots).ok())
            .collect()
    }

    /// Recursively count readable PDF files in configured roots.
    #[must_use]
    pub fn count_pdfs(roots: &[PathBuf]) -> u64 {
        let mut seen = std::collections::HashSet::new();
        roots
            .iter()
            .flat_map(|root| WalkDir::new(root).follow_links(false).into_iter())
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_pdf(entry.path()))
            // 翻訳成果物は論文として数えない（scan と件数を一致させる）
            .filter(|entry| !is_translated_pdf(entry.path()))
            .filter(|entry| {
                let p = std::fs::canonicalize(entry.path())
                    .unwrap_or_else(|_| entry.path().to_path_buf());
                seen.insert(p)
            })
            .count() as u64
    }

    /// Recursively calculate total size of readable PDF files in configured roots.
    #[must_use]
    pub fn pdf_storage_size(roots: &[PathBuf]) -> u64 {
        let mut seen = std::collections::HashSet::new();
        roots
            .iter()
            .flat_map(|root| WalkDir::new(root).follow_links(false).into_iter())
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_pdf(entry.path()))
            // 翻訳成果物は容量にも数えない（count_pdfs と件数・容量を一致させる）
            .filter(|entry| !is_translated_pdf(entry.path()))
            .filter(|entry| {
                let p = std::fs::canonicalize(entry.path())
                    .unwrap_or_else(|_| entry.path().to_path_buf());
                seen.insert(p)
            })
            .map(|entry| entry.metadata().map_or(0, |m| m.len()))
            .sum()
    }

    /// Recursively count PDF files and calculate their total size in one pass.
    ///
    /// This preserves the count and deduplication semantics of [`Self::count_pdfs`]
    /// while avoiding a second traversal when both values are needed together.
    #[must_use]
    pub fn pdf_storage_stats(roots: &[PathBuf]) -> (u64, u64) {
        let mut seen = std::collections::HashSet::new();
        roots
            .iter()
            .flat_map(|root| WalkDir::new(root).follow_links(false).into_iter())
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_pdf(entry.path()))
            // 翻訳成果物は件数・容量に数えない（scan / count_pdfs と一致させる）
            .filter(|entry| !is_translated_pdf(entry.path()))
            .filter(|entry| {
                let p = std::fs::canonicalize(entry.path())
                    .unwrap_or_else(|_| entry.path().to_path_buf());
                seen.insert(p)
            })
            .fold((0_u64, 0_u64), |(count, bytes), entry| {
                (
                    count + 1,
                    bytes + entry.metadata().map_or(0, |metadata| metadata.len()),
                )
            })
    }

    /// Discover every subdirectory represented as a filesystem collection.
    #[must_use]
    pub fn collection_directories(roots: &[PathBuf]) -> Vec<CollectionDirectory> {
        roots
            .iter()
            .flat_map(|root| {
                WalkDir::new(root)
                    .min_depth(1)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_dir())
                    .filter_map(|entry| {
                        entry
                            .path()
                            .strip_prefix(root)
                            .ok()
                            .map(|relative| CollectionDirectory {
                                library_root: root.clone(),
                                relative_path: relative.to_path_buf(),
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Hash and describe one PDF.
    ///
    /// # Errors
    ///
    /// Returns an error when the file metadata or contents cannot be read.
    pub fn inspect(path: &Path) -> Result<ImportedPdf, LibraryError> {
        let mut file = File::open(path)?;
        let file_size = file.metadata()?.len();
        let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let title = path
            .file_stem()
            .and_then(|name| name.to_str())
            .map_or_else(|| "Untitled PDF".to_owned(), humanize_filename);

        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        let content_hash = format!("{:x}", hasher.finalize());

        Ok(ImportedPdf {
            path: canonical_path,
            title,
            content_hash,
            file_size,
            library_root: None,
            relative_directory: None,
        })
    }

    /// Inspect a PDF and classify it relative to the most specific library root.
    ///
    /// # Errors
    /// Returns an error when the PDF cannot be inspected.
    pub fn inspect_in_roots(path: &Path, roots: &[PathBuf]) -> Result<ImportedPdf, LibraryError> {
        let mut pdf = Self::inspect(path)?;
        let canonical_roots: Vec<PathBuf> = roots
            .iter()
            .map(|r| std::fs::canonicalize(r).unwrap_or_else(|_| r.clone()))
            .collect();
        if let Some((root, canonical_root)) = roots
            .iter()
            .zip(canonical_roots.iter())
            .filter(|(_, canonical_root)| pdf.path.starts_with(canonical_root))
            .max_by_key(|(_, canonical_root)| canonical_root.components().count())
        {
            pdf.library_root = Some(root.clone());
            pdf.relative_directory = pdf
                .path
                .parent()
                .and_then(|parent| parent.strip_prefix(canonical_root).ok())
                .filter(|relative| !relative.as_os_str().is_empty())
                .map(Path::to_path_buf);
        }
        Ok(pdf)
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

/// PDF 翻訳（pdf2zh）で生成される翻訳成果物かどうかを判定する。
/// 翻訳済み PDF は `<stem>-dual.pdf`（対訳版）/ `<stem>-ja.pdf`（日本語版）/
/// `<stem>-mono.pdf`（pdf2zh が名前を変える前のモノクロ日本語版）の命名規則を持つ。
/// これらは元論文の一部であり、独立した論文として取り込まれるべきではない。
fn is_translated_pdf(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    // 例: "foo-dual.pdf" → stem "foo-dual"、"foo-mono.pdf" → stem "foo-mono"
    stem.ends_with("-dual") || stem.ends_with("-ja") || stem.ends_with("-mono")
}

fn humanize_filename(name: &str) -> String {
    name.replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use super::LibraryIndexer;

    #[test]
    fn inspect_hashes_and_humanizes_pdf() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("papr-index-{}.pdf", std::process::id()));
        fs::write(&path, b"%PDF-1.7\ntest")?;
        let imported = LibraryIndexer::inspect(&path)?;
        assert_eq!(imported.file_size, 13);
        assert!(imported.title.starts_with("papr index"));
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn missing_roots_produce_empty_scan() {
        let roots = [PathBuf::from("/path/that/does/not/exist/papr")];
        assert!(LibraryIndexer::scan(&roots).is_empty());
    }

    #[test]
    fn classifies_root_and_nested_pdfs() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-library-layout-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let collection = root.join("Gravitational Waves");
        fs::create_dir_all(&collection)?;
        let root_pdf = root.join("unassigned.pdf");
        let nested_pdf = collection.join("assigned.pdf");
        fs::create_dir_all(root.join("Empty Collection"))?;
        fs::write(&root_pdf, b"%PDF root")?;
        fs::write(&nested_pdf, b"%PDF nested")?;

        let root_item = LibraryIndexer::inspect_in_roots(&root_pdf, std::slice::from_ref(&root))?;
        let nested_item =
            LibraryIndexer::inspect_in_roots(&nested_pdf, std::slice::from_ref(&root))?;
        assert_eq!(root_item.library_root.as_deref(), Some(root.as_path()));
        assert_eq!(root_item.relative_directory, None);
        assert_eq!(
            nested_item.relative_directory,
            Some(PathBuf::from("Gravitational Waves"))
        );
        let directories = LibraryIndexer::collection_directories(std::slice::from_ref(&root));
        assert!(
            directories
                .iter()
                .any(|directory| { directory.relative_path == Path::new("Empty Collection") })
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn nested_roots_do_not_double_count() -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("papr-nested-roots-test-{}", std::process::id()));
        let sub = root.join("downloads");
        fs::create_dir_all(&sub)?;
        let pdf = sub.join("paper.pdf");
        fs::write(&pdf, b"%PDF-1.4 paper")?;

        let roots = vec![root.clone(), sub.clone()];
        let scanned = LibraryIndexer::scan(&roots);
        assert_eq!(scanned.len(), 1);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn storage_stats_match_individual_count_and_size_walks()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-storage-stats-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let nested = root.join("nested");
        fs::create_dir_all(&nested)?;
        fs::write(root.join("first.pdf"), b"%PDF first")?;
        fs::write(nested.join("second.pdf"), b"%PDF second")?;

        let roots = vec![root.clone(), nested];
        assert_eq!(
            LibraryIndexer::pdf_storage_stats(&roots),
            (
                LibraryIndexer::count_pdfs(&roots),
                LibraryIndexer::pdf_storage_size(&roots),
            )
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    /// `-mono.pdf`（pdf2zh が名前を変える前の翻訳 PDF）が scan・count_pdfs・
    /// 容量の集計から外れること（段階 1）。翻訳 PDF が論文として増えない。
    #[test]
    fn translated_mono_pdf_is_excluded_from_scan_and_counts(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "papr-mono-exclusion-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root)?;
        fs::write(root.join("original.pdf"), b"%PDF original")?;
        fs::write(root.join("original-mono.pdf"), b"%PDF mono translation data")?;
        fs::write(root.join("original-ja.pdf"), b"%PDF ja translation")?;
        fs::write(root.join("original-dual.pdf"), b"%PDF dual translation")?;

        let roots = [root.clone()];
        let scanned = LibraryIndexer::scan(&roots);
        assert_eq!(scanned.len(), 1, "翻訳 PDF が論文としてスキャンされている");
        assert_eq!(scanned[0].path.file_name().unwrap(), "original.pdf");
        assert_eq!(LibraryIndexer::count_pdfs(&roots), 1);
        assert_eq!(
            LibraryIndexer::pdf_storage_size(&roots),
            b"%PDF original".len() as u64
        );
        assert_eq!(
            LibraryIndexer::pdf_storage_stats(&roots),
            (1, b"%PDF original".len() as u64)
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
