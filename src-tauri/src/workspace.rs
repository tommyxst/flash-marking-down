use std::ffi::{OsStr, c_void};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::mem::{MaybeUninit, offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::RwLock;

pub(crate) const STATE_DIR: &str = ".flash-marking-down";
const STATE_DB_NAME: &str = "state.sqlite";
const LEASE_NAME: &str = "workspace.lock";
const SHARE_READ: u32 = 0x0000_0001;
const SHARE_WRITE: u32 = 0x0000_0002;
const LIST_DIRECTORY: u32 = 0x0000_0001;
const READ_ATTRIBUTES: u32 = 0x0000_0080;
const DELETE_ACCESS: u32 = 0x0001_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const REPARSE_POINT: u32 = 0x0000_0400;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_RENAME_INFO: i32 = 3;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

struct CaptureTarget {
    path: PathBuf,
    identity: [u32; 3],
    _pins: Vec<File>,
}

pub(crate) struct WorkspaceFiles {
    root: PathBuf,
    capture: RwLock<CaptureTarget>,
    database: PathBuf,
    state_identity: [u32; 3],
    file_identity: String,
    _lease: File,
    _database_pin: File,
    _directory_pins: Vec<File>,
}

pub(crate) struct MarkdownEntry {
    pub(crate) relative_path: String,
    pub(crate) mtime_ns: i64,
    pub(crate) file_size: i64,
}

pub(crate) struct MarkdownFile {
    pub(crate) file: File,
    path: PathBuf,
    _directory_pins: Vec<File>,
}

pub(crate) struct NewMarkdown {
    pub(crate) key: String,
    path: PathBuf,
    _directory_pins: Vec<File>,
}

impl NewMarkdown {
    pub(crate) fn create(&self) -> io::Result<File> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(&self.path)?;
        file.sync_all()?;
        Ok(file)
    }
}

impl WorkspaceFiles {
    pub(crate) fn open(
        root: &Path,
        capture_dir: &Path,
        state_dir_name: &str,
        expected_identity: Option<&str>,
    ) -> io::Result<Self> {
        validate_component(OsStr::new(state_dir_name))?;
        let (root, mut pins) = pin_workspace_root(root)?;
        let root_id = file_details(pins.last().expect("workspace root is pinned"))?.identity();
        let create = expected_identity.is_none();

        let mut capture_pins = Vec::new();
        let capture = pin_capture_directory(&root, capture_dir, &mut capture_pins)?;
        let capture_identity = file_details(
            capture_pins
                .last()
                .unwrap_or_else(|| pins.last().expect("workspace root is pinned")),
        )?
        .identity();
        let state = root.join(state_dir_name);
        if create {
            match fs::create_dir(&state) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        let state_pin = pin_directory(&state)?;
        let state_identity = file_details(&state_pin)?.identity();
        for directory in &capture_pins {
            if file_details(directory)?.identity() == state_identity {
                return Err(scope_error(
                    "capture path is inside the application state folder",
                ));
            }
        }
        pins.push(state_pin);

        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .share_mode(SHARE_READ | SHARE_WRITE)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(state.join(LEASE_NAME))?;
        validate_regular_file(&lease)?;
        lease.try_lock().map_err(io::Error::from)?;

        let database = state.join(STATE_DB_NAME);
        let database_pin = pin_database(&database, create)?;
        let database_id = file_details(&database_pin)?.identity();
        let file_identity: String = root_id
            .into_iter()
            .chain(database_id)
            .map(|part| format!("{part:08x}"))
            .collect();
        if expected_identity.is_some_and(|expected| expected != file_identity) {
            return Err(scope_error("workspace root or database has been replaced"));
        }
        for suffix in ["-journal", "-wal", "-shm"] {
            let sidecar = state.join(format!("{STATE_DB_NAME}{suffix}"));
            if let Some(file) = open_existing(&sidecar, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE)?
            {
                validate_regular_file(&file)?;
            }
        }

        Ok(Self {
            root,
            capture: RwLock::new(CaptureTarget {
                path: capture,
                identity: capture_identity,
                _pins: capture_pins,
            }),
            database,
            state_identity,
            file_identity,
            _lease: lease,
            _database_pin: database_pin,
            _directory_pins: pins,
        })
    }

    pub(crate) fn root_path(&self) -> &Path {
        &self.root
    }

    pub(crate) fn update_capture_dir(&self, capture_dir: &Path) -> io::Result<()> {
        let mut capture_pins = Vec::new();
        let capture = pin_capture_directory(&self.root, capture_dir, &mut capture_pins)?;
        for directory in &capture_pins {
            if file_details(directory)?.identity() == self.state_identity {
                return Err(scope_error(
                    "capture path is inside the application state folder",
                ));
            }
        }
        let identity = if let Some(last) = capture_pins.last() {
            file_details(last)?.identity()
        } else {
            let root_pin = pin_directory(&self.root)?;
            file_details(&root_pin)?.identity()
        };
        let mut guard = self
            .capture
            .write()
            .map_err(|_| io::Error::other("workspace capture lock poisoned"))?;
        *guard = CaptureTarget {
            path: capture,
            identity,
            _pins: capture_pins,
        };
        Ok(())
    }

    pub(crate) fn markdown_full_path(&self, relative_path: &str) -> io::Result<PathBuf> {
        let target = self.open_markdown(relative_path, GENERIC_READ, SHARE_READ)?;
        let canonical = target.path.canonicalize()?;
        canonical
            .strip_prefix(&self.root)
            .map_err(|_| scope_error("Markdown is outside the workspace"))?;
        Ok(canonical)
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database
    }

    pub(crate) fn file_identity(&self) -> &str {
        &self.file_identity
    }

    pub(crate) fn matches_root(&self, root: &Path) -> io::Result<bool> {
        let (root, _pins) = pin_workspace_root(root)?;
        Ok(root == self.root)
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
        let mut entries = Vec::new();
        let mut skipped = 0;
        self.collect_markdown(Path::new(""), &mut entries, &mut skipped)?;
        Ok((entries, skipped))
    }

    fn collect_markdown(
        &self,
        relative_dir: &Path,
        entries: &mut Vec<MarkdownEntry>,
        skipped: &mut usize,
    ) -> io::Result<()> {
        let directory = self.root.join(relative_dir);
        let pin = pin_directory(&directory)?;
        let identity = file_details(&pin)?.identity();
        if identity == self.state_identity {
            return Ok(());
        }
        // Use the registered capture directory spelling even for a short name or case alias.
        let capture_relative = {
            let guard = self
                .capture
                .read()
                .map_err(|_| io::Error::other("workspace capture lock poisoned"))?;
            (identity == guard.identity).then(|| {
                guard
                    .path
                    .strip_prefix(&self.root)
                    .expect("capture is within root")
                    .to_path_buf()
            })
        };
        let relative_dir = capture_relative.as_deref().unwrap_or(relative_dir);
        for entry in fs::read_dir(&directory)? {
            let result = entry.and_then(|entry| {
                let metadata = entry.metadata()?;
                if metadata.file_attributes() & REPARSE_POINT != 0 {
                    return Ok(());
                }
                let name = entry.file_name();
                validate_component(&name)?;
                let path = relative_dir.join(name);
                if metadata.is_dir() {
                    self.collect_markdown(&path, entries, skipped)?;
                } else if metadata.is_file() && is_markdown(&path) {
                    let text = path
                        .to_str()
                        .ok_or_else(|| scope_error("path is not Unicode"))?;
                    entries.push(MarkdownEntry {
                        relative_path: text.replace('\\', "/"),
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
        let mut target = self.open_markdown(relative_path, GENERIC_READ, SHARE_READ)?;
        let mut bytes = Vec::new();
        target.file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub(crate) fn read_markdown_with_identity(
        &self,
        relative_path: &str,
    ) -> io::Result<(Vec<u8>, String)> {
        let mut target = self.open_markdown(relative_path, GENERIC_READ, SHARE_READ)?;
        let identity = file_details(&target.file)?
            .identity()
            .into_iter()
            .map(|part| format!("{part:08x}"))
            .collect();
        let mut bytes = Vec::new();
        target.file.read_to_end(&mut bytes)?;
        Ok((bytes, identity))
    }

    pub(crate) fn markdown_key(&self, relative_path: &str) -> io::Result<String> {
        let target = self.open_markdown(relative_path, GENERIC_READ, SHARE_READ)?;
        let canonical = target.path.canonicalize()?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| scope_error("Markdown is outside the workspace"))?;
        Ok(relative
            .to_str()
            .ok_or_else(|| scope_error("path is not Unicode"))?
            .replace('\\', "/"))
    }

    pub(crate) fn lock_markdown(&self, relative_path: &str) -> io::Result<MarkdownFile> {
        self.open_markdown(relative_path, GENERIC_READ | GENERIC_WRITE, 0)
    }

    pub(crate) fn new_markdown(&self, relative_path: &str) -> io::Result<NewMarkdown> {
        let (path, pins) = self.pinned_markdown_path(relative_path)?;
        let parent = path
            .parent()
            .expect("validated Markdown file has a parent")
            .canonicalize()?;
        let key = parent
            .strip_prefix(&self.root)
            .map_err(|_| scope_error("target outside workspace"))?
            .join(
                path.file_name()
                    .expect("validated Markdown file has a name"),
            );
        let key = key
            .to_str()
            .ok_or_else(|| scope_error("path is not Unicode"))?
            .replace('\\', "/");
        Ok(NewMarkdown {
            key,
            path,
            _directory_pins: pins,
        })
    }

    fn open_markdown(
        &self,
        relative_path: &str,
        access: u32,
        share: u32,
    ) -> io::Result<MarkdownFile> {
        let (path, pins) = self.pinned_markdown_path(relative_path)?;
        let file = open_existing(&path, access, share)?
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "Markdown file is missing"))?;
        validate_regular_file(&file)?;
        Ok(MarkdownFile {
            file,
            path,
            _directory_pins: pins,
        })
    }

    fn pinned_markdown_path(&self, relative_path: &str) -> io::Result<(PathBuf, Vec<File>)> {
        if !is_markdown(Path::new(relative_path)) {
            return Err(scope_error("expected a Markdown file"));
        }
        let mut directory = self.root.clone();
        let mut pins = Vec::new();
        let mut components = relative_path.split('/').peekable();
        while let Some(name) = components.next() {
            validate_component(OsStr::new(name))?;
            directory.push(name);
            if components.peek().is_some() {
                let pin = pin_directory(&directory)?;
                if file_details(&pin)?.identity() == self.state_identity {
                    return Err(scope_error(
                        "application state is outside the document scope",
                    ));
                }
                pins.push(pin);
            }
        }
        Ok((directory, pins))
    }

    pub(crate) fn publish_record(&self, relative_path: &str, bytes: &[u8]) -> io::Result<File> {
        let (target, _pins) = self.pinned_markdown_path(relative_path)?;
        let file_name = target
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| scope_error("invalid record filename"))?;
        if let Some(mut file) = open_existing(&target, GENERIC_READ, SHARE_READ)? {
            verify_contents(&mut file, bytes)?;
            return Ok(file);
        }

        let stage = target
            .parent()
            .expect("validated record target has parent")
            .join(format!(".{file_name}.pending"));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE_ACCESS)
            .share_mode(SHARE_READ)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(stage)?;
        validate_regular_file(&file)?;

        // An interrupted write may leave a prefix. Preserve the file if any byte differs.
        let staged = read_bytes(&mut file, bytes.len())?;
        let remaining = bytes
            .strip_prefix(staged.as_slice())
            .ok_or_else(|| conflict_error("staged file has different content"))?;
        file.write_all(remaining)?;
        file.sync_all()?;
        rename_new_file(&file, &target)?;
        verify_contents(&mut file, bytes)?;
        Ok(file)
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        && !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("~$"))
}

pub(crate) fn pin_workspace_root(root: &Path) -> io::Result<(PathBuf, Vec<File>)> {
    let mut components = root.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(scope_error(
            "workspace requires an absolute local drive path",
        ));
    };
    if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err(scope_error(
            "workspace requires an absolute local drive path",
        ));
    }

    let mut path = PathBuf::from(prefix.as_os_str());
    path.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    let drive_path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // The buffer is NUL-terminated and lives through the system call.
    let drive_type = unsafe { GetDriveTypeW(drive_path.as_ptr()) };
    if !matches!(drive_type, DRIVE_FIXED | DRIVE_REMOVABLE) {
        return Err(scope_error("workspace requires a local drive"));
    }

    let mut pins = vec![pin_directory(&path)?];
    for component in components {
        let Component::Normal(name) = component else {
            return Err(scope_error("workspace path contains a relative component"));
        };
        validate_component(name)?;
        path.push(name);
        pins.push(pin_directory(&path)?);
    }
    Ok((path.canonicalize()?, pins))
}

fn pin_capture_directory(
    root: &Path,
    relative: &Path,
    pins: &mut Vec<File>,
) -> io::Result<PathBuf> {
    let mut capture = root.to_owned();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                validate_component(name)?;
                capture.push(name);
                pins.push(pin_directory(&capture)?);
            }
            _ => return Err(scope_error("capture path must stay inside the workspace")),
        }
    }
    Ok(capture)
}

pub(crate) fn capture_picker_path(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut pins = Vec::new();
    let capture = pin_capture_directory(root, relative, &mut pins)?;
    if let Some(first_child) = pins.first() {
        match pin_directory(&root.join(STATE_DIR)) {
            Ok(state) => {
                if file_details(first_child)?.identity() == file_details(&state)?.identity() {
                    return Err(scope_error(
                        "capture path is inside the application state folder",
                    ));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    capture.canonicalize()
}

pub(crate) fn picked_capture_directory(
    root: &Path,
    current: &str,
    selected: &Path,
) -> io::Result<String> {
    let (root, _root_pins) = pin_workspace_root(root)?;
    let (selected, _selected_pins) = pin_workspace_root(selected)?;
    let relative = selected
        .strip_prefix(&root)
        .map_err(|_| scope_error("capture path must stay inside the workspace"))?;
    if relative.as_os_str().is_empty() {
        return Ok(String::new());
    }
    capture_picker_path(&root, relative)?;
    if capture_picker_path(&root, Path::new(current)).is_ok_and(|path| path == selected) {
        return Ok(current.to_owned());
    }
    Ok(relative
        .to_str()
        .ok_or_else(|| scope_error("capture path is not Unicode"))?
        .replace('\\', "/"))
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

fn pin_directory(path: &Path) -> io::Result<File> {
    // Denying delete-sharing keeps every checked directory name in place while the workspace is open.
    let directory = OpenOptions::new()
        .access_mode(LIST_DIRECTORY | READ_ATTRIBUTES)
        .share_mode(SHARE_READ | SHARE_WRITE)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(scope_error(
            "workspace folders must be ordinary directories",
        ));
    }
    Ok(directory)
}

fn pin_database(path: &Path, create: bool) -> io::Result<File> {
    if create {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(path)
        {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let file = open_existing(path, READ_ATTRIBUTES, SHARE_READ | SHARE_WRITE)?
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "state database disappeared"))?;
    validate_regular_file(&file)?;
    Ok(file)
}

fn open_existing(path: &Path, access: u32, share: u32) -> io::Result<Option<File>> {
    match OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn validate_regular_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || file_details(file)?.number_of_links != 1
    {
        return Err(scope_error("expected a regular file without links"));
    }
    Ok(())
}

fn verify_contents(file: &mut File, expected: &[u8]) -> io::Result<()> {
    validate_regular_file(file)?;
    file.seek(SeekFrom::Start(0))?;
    if read_bytes(file, expected.len())? != expected {
        return Err(conflict_error("target file has different content"));
    }
    Ok(())
}

fn read_bytes(file: &mut File, expected_len: usize) -> io::Result<Vec<u8>> {
    let mut contents = Vec::new();
    file.take(expected_len as u64 + 1)
        .read_to_end(&mut contents)?;
    Ok(contents)
}

#[repr(C)]
struct FileDetails {
    attributes: u32,
    creation_time: [u32; 2],
    last_access_time: [u32; 2],
    last_write_time: [u32; 2],
    volume_serial_number: u32,
    size_high: u32,
    size_low: u32,
    number_of_links: u32,
    index_high: u32,
    index_low: u32,
}

impl FileDetails {
    fn identity(&self) -> [u32; 3] {
        [self.volume_serial_number, self.index_high, self.index_low]
    }
}

fn file_details(file: &File) -> io::Result<FileDetails> {
    let mut details = MaybeUninit::<FileDetails>::uninit();
    // The File supplies a live handle; the system initializes the output on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), details.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { details.assume_init() })
}

#[repr(C)]
struct FileRenameInfo {
    replace_existing: u32,
    root_directory: RawHandle,
    file_name_length: u32,
    file_name: [u16; 1],
}

fn rename_new_file(file: &File, target: &Path) -> io::Result<()> {
    let path: Vec<u16> = target.as_os_str().encode_wide().collect();
    let path_bytes = path.len() * size_of::<u16>();
    let byte_len = size_of::<FileRenameInfo>() + path_bytes;
    let mut buffer = vec![0usize; byte_len.div_ceil(size_of::<usize>())];
    let info = buffer.as_mut_ptr().cast::<FileRenameInfo>();

    // FILE_RENAME_INFO ends in a variable-length UTF-16 name. The aligned buffer
    // holds the header and all name bytes; a zero flag forbids replacing the target.
    let status = unsafe {
        (*info).replace_existing = 0;
        (*info).file_name_length = path_bytes as u32;
        let name = buffer
            .as_mut_ptr()
            .cast::<u8>()
            .add(offset_of!(FileRenameInfo, file_name));
        std::ptr::copy_nonoverlapping(path.as_ptr(), name.cast::<u16>(), path.len());
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FILE_RENAME_INFO,
            info.cast(),
            byte_len as u32,
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn scope_error(message: &'static str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message)
}

fn conflict_error(message: &'static str) -> io::Error {
    io::Error::new(ErrorKind::AlreadyExists, message)
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

        assert_eq!(picked_capture_directory(&root, "笔记", &root).unwrap(), "");
        assert_eq!(picked_capture_directory(&root, ".", &root).unwrap(), "");
        assert_eq!(
            picked_capture_directory(&root, "", &capture).unwrap(),
            "笔记/随手记"
        );
        assert_eq!(
            picked_capture_directory(&root, ".\\笔记\\随手记", &capture).unwrap(),
            ".\\笔记\\随手记"
        );
        assert!(!root.join(STATE_DIR).exists());
        fs::create_dir_all(&state).unwrap();
        assert!(picked_capture_directory(&root, "", &state).is_err());
        assert!(picked_capture_directory(&root, "", &outside).is_err());
    }
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetFileInformationByHandle(file: RawHandle, details: *mut FileDetails) -> i32;
    fn SetFileInformationByHandle(
        file: RawHandle,
        class: i32,
        info: *const c_void,
        len: u32,
    ) -> i32;
}
