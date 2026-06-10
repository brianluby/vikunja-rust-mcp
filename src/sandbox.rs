//! Attachment path sandboxing.
//!
//! Restricts the server-local file paths the attachment tools may touch.
//! Operators configure allowed root directories for uploads (files read
//! from disk) and downloads (files written to disk); paths outside those
//! roots are rejected after full canonicalization, so `..` traversal and
//! symlink escapes cannot leave a root. With no roots configured the
//! sandbox is permissive and behavior is unchanged.
//!
//! Known limitations (inherent to path-check sandboxes): a hard link
//! inside a root can alias a file outside it — hard links carry no marker
//! a check could detect — and a symlink swapped in between the check and
//! the subsequent IO (a TOCTOU race) is not caught. Both require an
//! attacker who can already write inside the configured root.

use std::path::{Path, PathBuf};

use crate::error::Error;

/// Validated attachment path sandbox. Roots are canonicalized absolute
/// paths; an empty root list disables checking for that direction.
#[derive(Debug, Clone, Default)]
pub struct AttachmentSandbox {
    upload_roots: Vec<PathBuf>,
    download_roots: Vec<PathBuf>,
}

impl AttachmentSandbox {
    /// Builds a sandbox from configured root directories, canonicalizing
    /// each one. Fails (closed) when a configured root does not exist or is
    /// not a directory, so a typo cannot silently disable the sandbox.
    pub fn new(upload_roots: &[PathBuf], download_roots: &[PathBuf]) -> Result<Self, InvalidRoot> {
        Ok(Self {
            upload_roots: canonicalize_roots(upload_roots)?,
            download_roots: canonicalize_roots(download_roots)?,
        })
    }

    /// True when no roots are configured in either direction.
    pub fn is_permissive(&self) -> bool {
        self.upload_roots.is_empty() && self.download_roots.is_empty()
    }

    /// Validates a path an upload wants to read and returns the canonical
    /// path to use for the read. The file must exist; symlinks are resolved
    /// before the root check, so a symlink inside a root pointing outside
    /// is rejected.
    pub fn resolve_upload_path(&self, raw: &str) -> Result<PathBuf, Error> {
        if self.upload_roots.is_empty() {
            return Ok(PathBuf::from(raw));
        }
        let canonical = Path::new(raw).canonicalize().map_err(|e| Error::Io {
            detail: format!("could not read {raw}: {e}"),
        })?;
        if !is_within(&canonical, &self.upload_roots) {
            return Err(Error::InvalidArgument(format!(
                "file_path `{raw}` is outside the allowed attachment upload directories"
            )));
        }
        Ok(canonical)
    }

    /// Validates a path a download wants to write and returns the path to
    /// use for the write: the canonicalized parent directory joined with
    /// the file name. The parent must exist and resolve into a download
    /// root; an existing symlink at the target is rejected so writes cannot
    /// be redirected outside the root.
    pub fn resolve_download_path(&self, raw: &str) -> Result<PathBuf, Error> {
        if self.download_roots.is_empty() {
            return Ok(PathBuf::from(raw));
        }
        let path = Path::new(raw);
        let Some(file_name) = path.file_name() else {
            return Err(Error::InvalidArgument(format!(
                "save_path `{raw}` does not name a file"
            )));
        };
        // Canonicalize the parent (it must exist) so `..` components and
        // symlinked directories resolve before the root check.
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        let canonical_parent = parent.canonicalize().map_err(|e| Error::Io {
            detail: format!("could not resolve the directory of {raw}: {e}"),
        })?;
        if !is_within(&canonical_parent, &self.download_roots) {
            return Err(Error::InvalidArgument(format!(
                "save_path `{raw}` is outside the allowed attachment download directories"
            )));
        }
        let candidate = canonical_parent.join(file_name);
        // Refuse to write through a pre-existing symlink: its target could
        // be outside the root even though the link itself is inside.
        if candidate
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(Error::InvalidArgument(format!(
                "save_path `{raw}` is an existing symlink; refusing to write through it"
            )));
        }
        Ok(candidate)
    }
}

/// A configured sandbox root that could not be validated.
#[derive(Debug, thiserror::Error)]
#[error("attachment root `{path}` is not usable: {reason}")]
pub struct InvalidRoot {
    pub path: String,
    pub reason: String,
}

fn canonicalize_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, InvalidRoot> {
    roots
        .iter()
        .map(|root| {
            let canonical = root.canonicalize().map_err(|e| InvalidRoot {
                path: root.display().to_string(),
                reason: e.to_string(),
            })?;
            if !canonical.is_dir() {
                return Err(InvalidRoot {
                    path: root.display().to_string(),
                    reason: "not a directory".to_string(),
                });
            }
            Ok(canonical)
        })
        .collect()
}

fn is_within(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(upload: &[&Path], download: &[&Path]) -> AttachmentSandbox {
        AttachmentSandbox::new(
            &upload.iter().map(PathBuf::from).collect::<Vec<_>>(),
            &download.iter().map(PathBuf::from).collect::<Vec<_>>(),
        )
        .expect("test roots should validate")
    }

    #[test]
    fn permissive_default_allows_any_path() {
        let sandbox = AttachmentSandbox::default();
        assert!(sandbox.is_permissive());
        assert_eq!(
            sandbox.resolve_upload_path("/anywhere/file.txt").unwrap(),
            PathBuf::from("/anywhere/file.txt")
        );
        assert_eq!(
            sandbox.resolve_download_path("/anywhere/out.txt").unwrap(),
            PathBuf::from("/anywhere/out.txt")
        );
    }

    #[test]
    fn missing_root_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let err = AttachmentSandbox::new(std::slice::from_ref(&missing), &[]).unwrap_err();
        assert!(err.path.contains("does-not-exist"));
    }

    #[test]
    fn file_as_root_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("root.txt");
        std::fs::write(&file, b"x").unwrap();
        let err = AttachmentSandbox::new(&[], &[file]).unwrap_err();
        assert!(err.reason.contains("not a directory"));
    }

    #[test]
    fn upload_inside_root_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.txt");
        std::fs::write(&file, b"data").unwrap();

        let sandbox = sandbox(&[dir.path()], &[]);
        let resolved = sandbox.resolve_upload_path(file.to_str().unwrap()).unwrap();
        assert_eq!(
            resolved,
            dir.path().canonicalize().unwrap().join("report.txt")
        );
    }

    #[test]
    fn sibling_directory_sharing_a_name_prefix_is_rejected() {
        // `starts_with` must compare whole path components: a root of
        // `<dir>/allowed` must not admit `<dir>/allowed-evil/...`.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("allowed");
        let sibling = dir.path().join("allowed-evil");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let file = sibling.join("secret.txt");
        std::fs::write(&file, b"data").unwrap();

        let sandbox = sandbox(&[&root], &[&root]);
        assert!(sandbox.resolve_upload_path(file.to_str().unwrap()).is_err());
        assert!(
            sandbox
                .resolve_download_path(sibling.join("out.bin").to_str().unwrap())
                .is_err()
        );
    }

    #[test]
    fn upload_outside_root_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        std::fs::write(&file, b"data").unwrap();

        let sandbox = sandbox(&[root.path()], &[]);
        let err = sandbox
            .resolve_upload_path(file.to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
        assert!(err.to_string().contains("upload"), "{err}");
        // The error must not enumerate the configured roots.
        assert!(!err.to_string().contains(root.path().to_str().unwrap()));
    }

    #[test]
    fn upload_traversal_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        std::fs::write(&file, b"data").unwrap();

        // root/../<outside>/secret.txt escapes via `..`.
        let sneaky = format!(
            "{}/../{}/secret.txt",
            root.path().to_str().unwrap(),
            outside.path().file_name().unwrap().to_str().unwrap()
        );
        let sandbox = sandbox(&[root.path()], &[]);
        let err = sandbox.resolve_upload_path(&sneaky).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn upload_symlink_escape_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, b"data").unwrap();
        let link = root.path().join("innocent.txt");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let sandbox = sandbox(&[root.path()], &[]);
        let err = sandbox
            .resolve_upload_path(link.to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn upload_of_missing_file_is_an_io_error() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[root.path()], &[]);
        let missing = root.path().join("missing.txt");
        let err = sandbox
            .resolve_upload_path(missing.to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err:?}");
    }

    #[test]
    fn upload_checks_second_root_too() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let file = second.path().join("ok.txt");
        std::fs::write(&file, b"data").unwrap();

        let sandbox = sandbox(&[first.path(), second.path()], &[]);
        assert!(sandbox.resolve_upload_path(file.to_str().unwrap()).is_ok());
    }

    #[test]
    fn download_to_new_file_inside_root_is_allowed() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[], &[root.path()]);
        let target = root.path().join("new-file.bin");
        let resolved = sandbox
            .resolve_download_path(target.to_str().unwrap())
            .unwrap();
        assert_eq!(
            resolved,
            root.path().canonicalize().unwrap().join("new-file.bin")
        );
    }

    #[test]
    fn download_outside_root_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[], &[root.path()]);
        let err = sandbox
            .resolve_download_path(outside.path().join("out.bin").to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
        assert!(err.to_string().contains("download"), "{err}");
        assert!(!err.to_string().contains(root.path().to_str().unwrap()));
    }

    #[test]
    fn download_traversal_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[], &[root.path()]);
        let sneaky = format!("{}/../escape.bin", root.path().to_str().unwrap());
        let err = sandbox.resolve_download_path(&sneaky).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn download_through_existing_symlink_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim.bin");
        std::fs::write(&target, b"old").unwrap();
        let link = root.path().join("alias.bin");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let sandbox = sandbox(&[], &[root.path()]);
        let err = sandbox
            .resolve_download_path(link.to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
        assert!(err.to_string().contains("symlink"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn download_into_symlinked_directory_outside_root_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link_dir = root.path().join("sub");
        std::os::unix::fs::symlink(outside.path(), &link_dir).unwrap();

        let sandbox = sandbox(&[], &[root.path()]);
        let err = sandbox
            .resolve_download_path(link_dir.join("out.bin").to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)), "{err:?}");
    }

    #[test]
    fn download_with_missing_parent_is_an_io_error() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[], &[root.path()]);
        let target = root.path().join("no-such-dir").join("out.bin");
        let err = sandbox
            .resolve_download_path(target.to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err:?}");
    }

    #[test]
    fn download_without_file_name_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[], &[root.path()]);
        for raw in ["..", "/"] {
            let err = sandbox.resolve_download_path(raw).unwrap_err();
            assert!(matches!(err, Error::InvalidArgument(_)), "{raw}: {err:?}");
        }
    }

    #[test]
    fn upload_roots_do_not_restrict_downloads_and_vice_versa() {
        let up = tempfile::tempdir().unwrap();
        let sandbox = sandbox(&[up.path()], &[]);
        // Download direction has no roots configured: stays permissive.
        let anywhere = tempfile::tempdir().unwrap();
        assert!(
            sandbox
                .resolve_download_path(anywhere.path().join("x.bin").to_str().unwrap())
                .is_ok()
        );
    }
}
