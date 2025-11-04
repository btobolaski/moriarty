//! SHA-256 hashing utilities for file and string content verification.
//!
//! This module provides utilities for computing SHA-256 hashes of files and strings,
//! which are used to verify that project tools configurations and executables haven't
//! been modified since they were approved.

use miette::{Context, IntoDiagnostic, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Computes the SHA-256 hash of a file's contents.
///
/// If the path is a symlink, it will be resolved before hashing.
///
/// # Arguments
///
/// * `path` - The path to the file to hash
///
/// # Returns
///
/// A hex-encoded SHA-256 hash string prefixed with "sha256:"
///
/// # Errors
///
/// Returns an error if:
/// - The path cannot be canonicalized
/// - The file cannot be read
/// - The file is a directory
pub async fn hash_file<P: AsRef<Path>>(path: P) -> Result<String> {
    let path = path.as_ref();

    let canonical_path = path
        .canonicalize()
        .into_diagnostic()
        .with_context(|| format!("Failed to canonicalize path: {}", path.display()))?;

    let contents = tokio::fs::read(&canonical_path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to read file: {}", canonical_path.display()))?;

    let mut hasher = Sha256::new();
    hasher.update(&contents);
    let hash = hasher.finalize();

    Ok(format!("sha256:{}", hex::encode(hash)))
}

/// Computes the SHA-256 hash of string content.
///
/// This is primarily used for hashing the contents of tools.toml configuration files.
///
/// # Arguments
///
/// * `content` - The string content to hash
///
/// # Returns
///
/// A hex-encoded SHA-256 hash string prefixed with "sha256:"
pub fn hash_string(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    format!("sha256:{}", hex::encode(hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_hash_file_simple() {
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "hello world").unwrap();
        temp_file.flush().unwrap();

        let hash = hash_file(temp_file.path()).await.unwrap();

        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn test_hash_file_empty() {
        let temp_file = NamedTempFile::new().unwrap();

        let hash = hash_file(temp_file.path()).await.unwrap();

        // SHA-256 of empty string
        assert_eq!(
            hash,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[tokio::test]
    async fn test_hash_file_nonexistent() {
        let result = hash_file("/nonexistent/file/path").await;
        let err = result.expect_err("Should fail for nonexistent file");
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("Failed to canonicalize path") || err_msg.contains("No such file") || err_msg.contains("not found"),
            "Error should mention file access failure, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_hash_string_simple() {
        let hash = hash_string("hello world");

        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_hash_string_empty() {
        let hash = hash_string("");

        // SHA-256 of empty string
        assert_eq!(
            hash,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_hash_string_multiline() {
        let content = "[commands]\nlint = [\"cargo\", \"clippy\"]\n";
        let hash = hash_string(content);

        // Hash should be consistent
        assert_eq!(hash, hash_string(content));

        // Hash should be different for different content
        let different_hash = hash_string("[commands]\ntest = [\"cargo\", \"test\"]\n");
        assert_ne!(hash, different_hash);
    }

    #[tokio::test]
    async fn test_hash_file_binary_content() {
        let mut temp_file = NamedTempFile::new().unwrap();
        // Write some binary content
        temp_file.write_all(&[0x00, 0xFF, 0x42, 0xAB]).unwrap();
        temp_file.flush().unwrap();

        let hash = hash_file(temp_file.path()).await.unwrap();

        // Hash should be computed correctly for binary data
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 71); // "sha256:" (7 chars) + 64 hex chars
    }
}
