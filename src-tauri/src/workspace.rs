use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const STATE_DIR: &str = ".flash-marking-down";
pub(crate) const DEFAULT_CAPTURE_DIR: &str = "notes";
const STATE_DB_NAME: &str = "state.sqlite";
const REPARSE_POINT: u32 = 0x0400;

pub(crate) struct WorkspaceFiles {
    root: PathBuf,
    state: PathBuf,
    database: PathBuf,
}

pub(crate) struct MarkdownEntry {
    pub(crate) relative_path: String,
    pub(crate) mtime_ns: i64,
    pub(crate) file_size: i64,
}

impl WorkspaceFiles {
    pub(crate) fn open(root: &Path, state_dir_name: &str) -> io::Result<Self> {
        validate_component(OsStr::new(state_dir_name))?;
        let root = workspace_root(root)?;
        let state = root.join(state_dir_name);
        match fs::create_dir(&state) {
            Ok(()) => (),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        let state = workspace_root(&state)?;
        if state == root || !state.starts_with(&root) {
            return Err(scope_error("state directory is outside the workspace"));
        }
        let mut database = state.join(STATE_DB_NAME);
        if let Some(existing) = existing_path(&database)? {
            if !existing.starts_with(&state) || !existing.is_file() {
                return Err(scope_error("state database is outside the state directory"));
            }
            database = existing;
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = database.as_os_str().to_os_string();
            sidecar.push(suffix);
            if existing_path(Path::new(&sidecar))?.is_some_and(|path| !path.starts_with(&state)) {
                return Err(scope_error(
                    "state database sidecar is outside the state directory",
                ));
            }
        }
        Ok(Self {
            root,
            state,
            database,
        })
    }

    pub(crate) fn root_path(&self) -> &Path {
        &self.root
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database
    }

    pub(crate) fn existing_database_path(&self) -> io::Result<&Path> {
        if self.database.canonicalize()? != self.database {
            return Err(scope_error("state database path changed"));
        }
        Ok(&self.database)
    }

    pub(crate) fn capture_key(&self, relative: &Path) -> io::Result<String> {
        validate_relative(relative)?;
        let relative = if relative.as_os_str().is_empty() || relative == Path::new(".") {
            Path::new(DEFAULT_CAPTURE_DIR)
        } else {
            relative
        };
        let current = self.ensure_directory(relative)?;
        if current == self.root {
            return Err(scope_error(
                "capture directory must be a subfolder inside the workspace",
            ));
        }
        self.key(&current)
    }

    fn ensure_directory(&self, relative: &Path) -> io::Result<PathBuf> {
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(segment) = component else {
                continue;
            };
            let next = current.join(segment);
            if !next.try_exists()? {
                match fs::create_dir(&next) {
                    Ok(()) => (),
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => (),
                    Err(error) => return Err(error),
                }
            }
            current = workspace_root(&next)?;
            self.key(&current)?;
        }
        Ok(current)
    }

    fn key(&self, resolved: &Path) -> io::Result<String> {
        let relative = in_workspace(&self.root, resolved)?;
        if resolved.starts_with(&self.state) {
            return Err(scope_error(
                "application state is outside the document scope",
            ));
        }
        relative_key(relative)
    }

    pub(crate) fn markdown_paths(&self) -> io::Result<(Vec<String>, usize)> {
        let (entries, skipped) = self.markdown_entries()?;
        Ok((
            entries
                .into_iter()
                .map(|entry| entry.relative_path)
                .collect(),
            skipped,
        ))
    }

    pub(crate) fn markdown_entries(&self) -> io::Result<(Vec<MarkdownEntry>, usize)> {
        if self.root.canonicalize()? != self.root {
            return Err(scope_error("workspace path changed"));
        }
        let mut entries = Vec::new();
        let mut seen = HashSet::from([self.root.clone()]);
        let mut skipped = 0;
        self.collect_markdown(&self.root, &mut seen, &mut entries, &mut skipped)?;
        Ok((entries, skipped))
    }

    fn collect_markdown(
        &self,
        directory: &Path,
        seen: &mut HashSet<PathBuf>,
        entries: &mut Vec<MarkdownEntry>,
        skipped: &mut usize,
    ) -> io::Result<()> {
        for result in fs::read_dir(directory)? {
            let result = result.and_then(|entry| {
                let name = entry.file_name();
                if name.eq_ignore_ascii_case(STATE_DIR) {
                    return Ok(());
                }
                let path = entry.path();
                let metadata = entry.metadata()?;
                let (path, metadata) = if metadata.file_attributes() & REPARSE_POINT != 0 {
                    let resolved = path.canonicalize()?;
                    self.key(&resolved)?;
                    let metadata = resolved.metadata()?;
                    (resolved, metadata)
                } else {
                    (path, metadata)
                };
                if path == self.state || !seen.insert(path.clone()) {
                    return Ok(());
                }
                if metadata.is_dir() {
                    self.collect_markdown(&path, seen, entries, skipped)?;
                } else if metadata.is_file() && is_markdown(&path) {
                    entries.push(MarkdownEntry {
                        relative_path: self.key(&path)?,
                        mtime_ns: metadata.last_write_time().min(i64::MAX as u64) as i64,
                        file_size: metadata.file_size().min(i64::MAX as u64) as i64,
                    });
                }
                Ok(())
            });
            if result.is_err() {
                *skipped += 1;
            }
        }
        Ok(())
    }

    pub(crate) fn read_markdown(&self, relative_path: &str) -> io::Result<Vec<u8>> {
        fs::read(self.markdown_full_path(relative_path)?)
    }

    pub(crate) fn read_local_image(
        &self,
        base_markdown: Option<&str>,
        image_ref: &str,
    ) -> io::Result<(&'static str, Vec<u8>)> {
        let trimmed = image_ref.trim();
        let raw_path = trimmed
            .split(['#', '?'])
            .next()
            .unwrap_or("")
            .trim()
            .replace('\\', "/");
        if raw_path.is_empty()
            || raw_path.starts_with("//")
            || raw_path.starts_with('/')
            || raw_path
                .split('/')
                .next()
                .is_some_and(|head| head.contains(':'))
        {
            return Err(scope_error("remote or absolute image paths are blocked"));
        }
        let rel = Path::new(&raw_path);
        for component in rel.components() {
            match component {
                Component::CurDir | Component::ParentDir => (),
                Component::Normal(name) if !name.eq_ignore_ascii_case(STATE_DIR) => {
                    validate_component(name)?;
                }
                _ => return Err(scope_error("image path is outside the document scope")),
            }
        }
        let base_dir = match base_markdown {
            Some(base) if !base.trim().is_empty() => {
                let physical = base.split_once("#fmd-note:").map_or(base, |(path, _)| path);
                let mut parent = self.root.clone();
                if let Some(dir) = Path::new(physical).parent() {
                    validate_relative(dir)?;
                    parent.push(dir);
                }
                parent
            }
            _ => self.root.clone(),
        };
        let resolved = base_dir.join(rel).canonicalize()?;
        self.key(&resolved)?;
        let meta = resolved.metadata()?;
        if !meta.is_file() {
            return Err(scope_error("expected a regular image file"));
        }
        const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "image exceeds 10 MiB limit",
            ));
        }
        let mime = match resolved
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref()
        {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            Some("svg") => "image/svg+xml",
            Some("ico") => "image/x-icon",
            _ => return Err(scope_error("unsupported image extension")),
        };
        Ok((mime, fs::read(resolved)?))
    }

    pub(crate) fn markdown_full_path(&self, relative_path: &str) -> io::Result<PathBuf> {
        self.markdown_file(relative_path).map(|(_, path)| path)
    }

    pub(crate) fn markdown_key(&self, relative_path: &str) -> io::Result<String> {
        self.markdown_file(relative_path).map(|(key, _)| key)
    }

    pub(crate) fn markdown_file(&self, relative_path: &str) -> io::Result<(String, PathBuf)> {
        self.picked_markdown(&self.markdown_input(relative_path)?)
    }

    pub(crate) fn picked_markdown(&self, selected: &Path) -> io::Result<(String, PathBuf)> {
        let path = selected.canonicalize()?;
        Ok((self.checked_markdown(&path)?, path))
    }

    pub(crate) fn markdown_target(&self, relative_path: &str) -> io::Result<(String, PathBuf)> {
        let requested = self.markdown_input(relative_path)?;
        if let Some(path) = existing_path(&requested)? {
            return Ok((self.checked_markdown(&path)?, path));
        }
        let parent = requested.parent().expect("Markdown path has a parent");
        let parent = workspace_root(parent)?;
        let path = parent.join(requested.file_name().expect("Markdown path has a name"));
        let key = self.key(&path)?;
        Ok((key, path))
    }

    pub(crate) fn create_markdown_target(
        &self,
        relative_path: &str,
    ) -> io::Result<(String, PathBuf)> {
        let requested = self.markdown_input(relative_path)?;
        if let Some(path) = existing_path(&requested)? {
            return Ok((self.checked_markdown(&path)?, path));
        }
        if self.root.canonicalize()? != self.root {
            return Err(scope_error("workspace path changed"));
        }
        let parent = Path::new(relative_path)
            .parent()
            .expect("Markdown path has a parent");
        let parent = self.ensure_directory(parent)?;
        let path = parent.join(requested.file_name().expect("Markdown path has a name"));
        Ok((self.key(&path)?, path))
    }

    fn checked_markdown(&self, path: &Path) -> io::Result<String> {
        let relative_path = self.key(path)?;
        if !is_markdown(path) || !path.metadata()?.is_file() {
            return Err(scope_error("expected a regular Markdown file"));
        }
        Ok(relative_path)
    }

    fn markdown_input(&self, relative_path: &str) -> io::Result<PathBuf> {
        let path = Path::new(relative_path);
        if !is_markdown(path) {
            return Err(scope_error("expected a Markdown file"));
        }
        validate_relative(path)?;
        Ok(self.root.join(path))
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        && !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("~$"))
}

pub(crate) fn workspace_root(root: &Path) -> io::Result<PathBuf> {
    if !root.is_absolute() {
        return Err(scope_error("workspace requires an absolute directory path"));
    }
    let path = root.canonicalize()?;
    if !path.metadata()?.is_dir() {
        return Err(scope_error("expected a directory"));
    }
    Ok(path)
}

pub(crate) fn capture_picker_path(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    validate_relative(relative)?;
    if relative.as_os_str().is_empty() || relative == Path::new(".") {
        return Ok(root.to_owned());
    }
    let capture = match workspace_root(&root.join(relative)) {
        Ok(path) => path,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(root.to_owned()),
        Err(error) => return Err(error),
    };
    in_workspace(root, &capture)?;
    Ok(capture)
}

pub(crate) fn picked_capture_directory(
    checked_root: &Path,
    current: &str,
    selected: &Path,
) -> io::Result<String> {
    let selected = workspace_root(selected)?;
    let relative = in_workspace(checked_root, &selected)?;
    if relative.as_os_str().is_empty() {
        return Err(scope_error(
            "capture directory must be a subfolder inside the workspace",
        ));
    }
    if capture_picker_path(checked_root, Path::new(current)).is_ok_and(|path| path == selected) {
        return Ok(current.to_owned());
    }
    relative_key(relative)
}

fn in_workspace<'a>(root: &Path, path: &'a Path) -> io::Result<&'a Path> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| scope_error("path is outside the workspace"))?;
    validate_relative(relative)?;
    Ok(relative)
}

fn relative_key(path: &Path) -> io::Result<String> {
    let relative = path
        .to_str()
        .ok_or_else(|| scope_error("path is not Unicode"))?;
    Ok(relative.replace('\\', "/"))
}

fn validate_relative(path: &Path) -> io::Result<()> {
    for component in path.components() {
        match component {
            Component::CurDir => (),
            Component::Normal(name) if !name.eq_ignore_ascii_case(STATE_DIR) => {
                validate_component(name)?
            }
            _ => return Err(scope_error("path is outside the document scope")),
        }
    }
    Ok(())
}

fn validate_component(name: &OsStr) -> io::Result<()> {
    let text = name
        .to_str()
        .ok_or_else(|| scope_error("path segment is not Unicode"))?;
    if text.is_empty()
        || text.ends_with([' ', '.'])
        || text
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err(scope_error("path segment is invalid"));
    }
    Ok(())
}

fn existing_path(path: &Path) -> io::Result<Option<PathBuf>> {
    match path.canonicalize() {
        Ok(resolved) => Ok(Some(resolved)),
        Err(error) if error.kind() == ErrorKind::NotFound => match path.symlink_metadata() {
            Err(missing) if missing.kind() == ErrorKind::NotFound => Ok(None),
            _ => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn scope_error(message: &'static str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message)
}

pub(crate) fn atomic_write(path: &Path, body: &[u8]) -> io::Result<()> {
    replace_file(path, body, || Ok(()))
}

pub(crate) fn replace_markdown(
    path: &Path,
    expected: Option<&[u8]>,
    body: &[u8],
) -> io::Result<()> {
    replace_file(path, body, || {
        let matching = match fs::read(path) {
            Ok(current) => expected == Some(current.as_slice()),
            Err(error) if error.kind() == ErrorKind::NotFound => expected.is_none(),
            Err(error) => return Err(error),
        };
        if matching {
            Ok(())
        } else {
            Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "Markdown changed on disk",
            ))
        }
    })
}

fn replace_file(
    path: &Path,
    body: &[u8],
    before_rename: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = path.with_file_name(format!(
        ".fmd-{}-{stamp}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        before_rename()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;

    #[test]
    fn picker_only_returns_capture_paths_inside_the_workspace() {
        let dir = TestDir::new();
        let root = dir.0.join("workspace");
        let outside = dir.0.join("workspace-other");
        let capture = root.join("笔记").join("随手记");
        let state = root.join(STATE_DIR).join("nested");
        fs::create_dir_all(&capture).unwrap();
        fs::create_dir(&outside).unwrap();
        let checked_root = workspace_root(&root).unwrap();

        for relative in ["", ".", "missing/daily"] {
            assert_eq!(
                capture_picker_path(&checked_root, Path::new(relative)).unwrap(),
                checked_root
            );
        }
        assert!(!root.join("missing").exists());
        assert!(picked_capture_directory(&checked_root, "笔记", &root).is_err());
        assert_eq!(
            picked_capture_directory(&checked_root, "", &capture).unwrap(),
            "笔记/随手记"
        );
        assert_eq!(
            picked_capture_directory(&checked_root, ".\\笔记\\随手记", &capture).unwrap(),
            ".\\笔记\\随手记"
        );
        assert!(!root.join(STATE_DIR).exists());
        fs::create_dir_all(&state).unwrap();
        assert!(picked_capture_directory(&checked_root, "", &state).is_err());
        assert!(picked_capture_directory(&checked_root, "", &outside).is_err());
    }

    #[test]
    fn reads_relative_workspace_images_and_blocks_remote_or_escaping_paths() {
        let dir = TestDir::new();
        let root = dir.0.join("workspace");
        let outside = dir.0.join("outside.png");
        fs::create_dir_all(root.join("daily/assets")).unwrap();
        fs::create_dir_all(root.join(STATE_DIR)).unwrap();
        fs::write(root.join("daily/assets/shot.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        fs::write(root.join(STATE_DIR).join("secret.png"), b"\x89PNG").unwrap();
        fs::write(&outside, b"\x89PNG").unwrap();

        let files = WorkspaceFiles::open(&root, STATE_DIR).unwrap();
        let (mime, bytes) = files
            .read_local_image(
                Some("daily/2026-11-25.md#fmd-note:abcd1234"),
                "./assets/shot.png",
            )
            .unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");

        for blocked in [
            "https://example.com/tracker.png",
            "http://localhost/x.png",
            "data:image/png;base64,AAAA",
            "../outside.png",
            "../../outside.png",
            "../.flash-marking-down/secret.png",
            ".flash-marking-down/secret.png",
            "assets/not-image.txt",
        ] {
            assert!(
                files
                    .read_local_image(Some("daily/2026-11-25.md"), blocked)
                    .is_err(),
                "expected `{blocked}` to be rejected"
            );
        }
    }
}
