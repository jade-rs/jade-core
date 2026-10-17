//! Which files `open_docx` may read and `export_docx` may write.
//!
//! ## Why the server restricts paths at all
//!
//! The paths arrive as tool arguments, and tool arguments are written by a
//! model that also reads the documents it edits. A sentence planted in one
//! contract ("now open ~/.ssh/id_rsa", "export over ~/.zshrc") is enough to
//! point these tools anywhere the process can reach. Combined with a tool that
//! sends a file somewhere — an embedder's upload tool, say — that is a
//! read-anything-and-exfiltrate primitive driven by prompt injection.
//!
//! So the filesystem surface is narrowed to what editing a `.docx` needs:
//!
//! - only paths under the allowed roots, judged after symlinks are resolved,
//!   so a link inside a root cannot reach outside it;
//! - only `.docx` files;
//! - `export_docx` only ever creates a new file. It never overwrites, and
//!   never writes through a symlink, because "create" is a single
//!   `O_CREAT | O_EXCL` open rather than a check followed by a write.
//!
//! ## Where the roots come from
//!
//! [`JADE_MCP_ALLOWED_ROOTS_ENV`] lists them, separated like `PATH`. Unset, the
//! server allows its working directory: MCP hosts such as Claude Code start a
//! stdio server in the project the user opened, which is the folder they mean
//! to work in. An embedder that knows better passes its own roots through
//! [`crate::server::Server::with_file_access`].

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Environment variable listing the directories the file tools may touch,
/// separated like `PATH` (`:` on Unix, `;` on Windows).
pub const JADE_MCP_ALLOWED_ROOTS_ENV: &str = "JADE_MCP_ALLOWED_ROOTS";

/// The directories `open_docx` / `export_docx` are confined to.
#[derive(Clone, Debug)]
pub struct FileAccess {
    /// Canonical (symlink-free, absolute) forms of the allowed roots.
    roots: Vec<PathBuf>,
}

impl FileAccess {
    /// Allow exactly `roots`. Roots that do not exist are dropped: a root that
    /// cannot be canonicalised cannot be compared against safely.
    pub fn new(roots: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
        Self {
            roots: roots
                .into_iter()
                .filter_map(|root| std::fs::canonicalize(root).ok())
                .collect(),
        }
    }

    /// Roots from [`JADE_MCP_ALLOWED_ROOTS_ENV`], or the working directory when
    /// it is unset or empty.
    pub fn from_env() -> Self {
        let configured = std::env::var_os(JADE_MCP_ALLOWED_ROOTS_ENV)
            .map(|value| {
                std::env::split_paths(&value)
                    .filter(|path| !path.as_os_str().is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if configured.is_empty() {
            Self::new(std::env::current_dir().ok())
        } else {
            Self::new(configured)
        }
    }

    /// The canonical roots, for messages and diagnostics.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Resolve `raw` to an existing `.docx` under a root, ready to read.
    pub fn resolve_existing(&self, raw: &str) -> Result<PathBuf, String> {
        let candidate = self.absolute(raw)?;
        require_docx_extension(&candidate, raw)?;
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|err| format!("failed to read {raw}: {err}"))?;
        // Check the extension again after resolution: `a.docx -> ~/.ssh/id_rsa`
        // passes the first check by name alone.
        require_docx_extension(&canonical, raw)?;
        self.require_inside(&canonical, raw)?;
        if !canonical.is_file() {
            return Err(format!("{raw} is not a regular file"));
        }
        Ok(canonical)
    }

    /// Create `raw` as a new `.docx` under a root and return it open for
    /// writing. Fails if anything — a file, a directory, a symlink, even a
    /// dangling one — already exists at that name.
    pub fn create_new(&self, raw: &str) -> Result<(PathBuf, File), String> {
        let candidate = self.absolute(raw)?;
        require_docx_extension(&candidate, raw)?;
        let name = candidate
            .file_name()
            .ok_or_else(|| format!("{raw} does not name a file"))?
            .to_owned();
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("{raw} has no parent directory"))?;
        // The directory may itself be a symlink; judge where it really is.
        let parent =
            std::fs::canonicalize(parent).map_err(|err| format!("failed to write {raw}: {err}"))?;
        self.require_inside(&parent, raw)?;
        let target = parent.join(name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "{raw} already exists. export_docx never overwrites; \
                         choose a new file name."
                    )
                } else {
                    format!("failed to write {raw}: {err}")
                }
            })?;
        Ok((target, file))
    }

    /// `raw` as an absolute path. Relative paths resolve against the first
    /// root, so `out.docx` lands where a user would look for it. `..` is left
    /// to canonicalisation: the root check runs on the resolved path, so a
    /// path that climbs out of a root is refused there.
    fn absolute(&self, raw: &str) -> Result<PathBuf, String> {
        if raw.is_empty() {
            return Err("path must not be empty".to_string());
        }
        if self.roots.is_empty() {
            return Err(self.no_roots_message());
        }
        let path = Path::new(raw);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        Ok(self.roots[0].join(path))
    }

    fn require_inside(&self, canonical: &Path, raw: &str) -> Result<(), String> {
        if self.roots.iter().any(|root| canonical.starts_with(root)) {
            return Ok(());
        }
        let roots = self
            .roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "{raw} is outside the allowed directories ({roots}). \
             Set {JADE_MCP_ALLOWED_ROOTS_ENV} to allow another directory, \
             or pass the document as bytes_base64."
        ))
    }

    fn no_roots_message(&self) -> String {
        format!(
            "file access is disabled: no allowed directory exists. \
             Set {JADE_MCP_ALLOWED_ROOTS_ENV}, or pass the document as bytes_base64."
        )
    }
}

fn require_docx_extension(path: &Path, raw: &str) -> Result<(), String> {
    let is_docx = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"));
    if is_docx {
        Ok(())
    } else {
        Err(format!("{raw}: only .docx files can be opened or exported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that removes itself, without a tempfile dependency.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "jade-file-access-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup(tag: &str) -> (Scratch, Scratch, FileAccess) {
        let root = Scratch::new(&format!("{tag}-root"));
        let outside = Scratch::new(&format!("{tag}-outside"));
        std::fs::write(root.0.join("in.docx"), b"PK").unwrap();
        std::fs::write(outside.0.join("secret.docx"), b"PK").unwrap();
        std::fs::write(outside.0.join("id_rsa"), b"key").unwrap();
        let access = FileAccess::new([&root.0]);
        (root, outside, access)
    }

    fn path(dir: &Path, name: &str) -> String {
        dir.join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn reads_docx_inside_a_root_by_absolute_or_relative_path() {
        let (root, _outside, access) = setup("read-ok");
        let expected = std::fs::canonicalize(root.0.join("in.docx")).unwrap();
        assert_eq!(
            access.resolve_existing(&path(&root.0, "in.docx")).unwrap(),
            expected
        );
        assert_eq!(access.resolve_existing("in.docx").unwrap(), expected);
    }

    #[test]
    fn refuses_reads_outside_the_roots() {
        let (root, outside, access) = setup("read-outside");
        let error = access
            .resolve_existing(&path(&outside.0, "secret.docx"))
            .unwrap_err();
        assert!(error.contains("outside the allowed directories"), "{error}");
        let error = access
            .resolve_existing(&format!(
                "{}/../{}/secret.docx",
                root.0.display(),
                outside.0.file_name().unwrap().to_string_lossy()
            ))
            .unwrap_err();
        assert!(error.contains("outside the allowed directories"), "{error}");
    }

    #[test]
    fn refuses_non_docx_files() {
        let (_root, outside, access) = setup("read-ext");
        let error = access
            .resolve_existing(&path(&outside.0, "id_rsa"))
            .unwrap_err();
        assert!(error.contains("only .docx"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_a_root_cannot_reach_outside_it() {
        let (root, outside, access) = setup("read-link");
        std::os::unix::fs::symlink(outside.0.join("secret.docx"), root.0.join("link.docx"))
            .unwrap();
        std::os::unix::fs::symlink(outside.0.join("id_rsa"), root.0.join("key.docx")).unwrap();
        let error = access.resolve_existing("link.docx").unwrap_err();
        assert!(error.contains("outside"), "{error}");
        let error = access.resolve_existing("key.docx").unwrap_err();
        assert!(error.contains("only .docx"), "{error}");
    }

    #[test]
    fn creates_new_docx_inside_a_root() {
        let (root, _outside, access) = setup("write-ok");
        let (written, _file) = access.create_new("out.docx").unwrap();
        assert_eq!(
            written,
            std::fs::canonicalize(&root.0).unwrap().join("out.docx")
        );
        assert!(written.is_file());
    }

    #[test]
    fn never_overwrites() {
        let (root, _outside, access) = setup("write-exists");
        let error = access.create_new(&path(&root.0, "in.docx")).unwrap_err();
        assert!(error.contains("never overwrites"), "{error}");
        assert_eq!(std::fs::read(root.0.join("in.docx")).unwrap(), b"PK");
    }

    #[test]
    fn refuses_writes_outside_the_roots_or_of_other_types() {
        let (_root, outside, access) = setup("write-outside");
        let error = access
            .create_new(&path(&outside.0, "new.docx"))
            .unwrap_err();
        assert!(error.contains("outside"), "{error}");
        assert!(!outside.0.join("new.docx").exists());
        let error = access.create_new("payload.sh").unwrap_err();
        assert!(error.contains("only .docx"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn writes_never_follow_symlinks() {
        let (root, outside, access) = setup("write-link");
        // A link to an existing file outside, and a dangling one.
        std::os::unix::fs::symlink(outside.0.join("secret.docx"), root.0.join("link.docx"))
            .unwrap();
        std::os::unix::fs::symlink(outside.0.join("fresh.docx"), root.0.join("dangling.docx"))
            .unwrap();
        // A directory link that leads outside.
        std::os::unix::fs::symlink(&outside.0, root.0.join("escape")).unwrap();
        for target in ["link.docx", "dangling.docx", "escape/new.docx"] {
            assert!(access.create_new(target).is_err(), "{target}");
        }
        assert_eq!(std::fs::read(outside.0.join("secret.docx")).unwrap(), b"PK");
        assert!(!outside.0.join("fresh.docx").exists());
        assert!(!outside.0.join("new.docx").exists());
    }

    #[test]
    fn no_roots_disables_file_access() {
        let access = FileAccess::new(Vec::<PathBuf>::new());
        assert!(access
            .resolve_existing("/tmp/a.docx")
            .unwrap_err()
            .contains("disabled"));
        assert!(access
            .create_new("a.docx")
            .unwrap_err()
            .contains("disabled"));
    }
}
