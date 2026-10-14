//! [`Archive`]: an opened archive with its central directory parsed once,
//! readable synchronously from any thread.

use std::collections::BTreeSet;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

use zip::ZipArchive;

/// The byte every archive byte is XOR-ed with when obfuscation is on.
/// Matches the single-byte default of the `xorio` crate used by 0.3, so
/// older obfuscated archives stay readable.
pub(crate) const XOR_BYTE: u8 = 0b0101_0101;

/// How to open an archive.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveOptions {
    pub obfuscate: bool,
    pub password: Option<String>,
}

impl ArchiveOptions {
    pub fn with_obfuscate(self, obfuscate: bool) -> Self {
        Self { obfuscate, ..self }
    }
    pub fn with_password(self, password: impl ToString) -> Self {
        Self {
            password: Some(password.to_string()),
            ..self
        }
    }
}

/// Where the archive bytes live.
enum Backing {
    #[cfg(not(target_arch = "wasm32"))]
    File(std::fs::File),
    Bytes(Arc<[u8]>),
}

impl Backing {
    fn len(&self) -> io::Result<u64> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Backing::File(file) => Ok(file.metadata()?.len()),
            Backing::Bytes(bytes) => Ok(bytes.len() as u64),
        }
    }

    /// Reads at `offset` without touching any shared cursor.
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Backing::File(file) => file_read_at(file, buf, offset),
            Backing::Bytes(bytes) => {
                let start = (offset as usize).min(bytes.len());
                let count = buf.len().min(bytes.len() - start);
                buf[..count].copy_from_slice(&bytes[start..start + count]);
                Ok(count)
            }
        }
    }
}

#[cfg(unix)]
fn file_read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn file_read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    // `seek_read` moves the handle's cursor, but every read here passes its
    // own offset, so concurrent readers never depend on it.
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(not(any(unix, windows, target_arch = "wasm32")))]
fn file_read_at(file: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(offset))?;
    file.read(buf)
}

/// A cheap-to-clone, position-independent reader over the archive bytes.
/// Every clone keeps its own cursor, so one parsed [`ZipArchive`] can be
/// cloned per read instead of re-parsing the central directory.
#[derive(Clone)]
pub(crate) struct ArchiveFile {
    backing: Arc<Backing>,
    len: u64,
    pos: u64,
    obfuscate: bool,
    /// Read-ahead window: `buf[buf_pos..]` holds the bytes at `pos`.
    buf: Vec<u8>,
    buf_pos: usize,
}

const READ_AHEAD: usize = 64 * 1024;

impl ArchiveFile {
    fn new(backing: Backing, obfuscate: bool) -> io::Result<Self> {
        let len = backing.len()?;
        Ok(Self {
            backing: Arc::new(backing),
            len,
            pos: 0,
            obfuscate,
            buf: Vec::new(),
            buf_pos: 0,
        })
    }

    fn fill(&mut self) -> io::Result<()> {
        self.buf.resize(READ_AHEAD, 0);
        let count = self.backing.read_at(&mut self.buf, self.pos)?;
        self.buf.truncate(count);
        self.buf_pos = 0;
        if self.obfuscate {
            self.buf.iter_mut().for_each(|byte| *byte ^= XOR_BYTE);
        }
        Ok(())
    }
}

impl Read for ArchiveFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.len {
            return Ok(0);
        }
        // Large reads (stored files) bypass the window.
        if self.buf_pos >= self.buf.len() && out.len() >= READ_AHEAD {
            let count = self.backing.read_at(out, self.pos)?;
            if self.obfuscate {
                out[..count].iter_mut().for_each(|byte| *byte ^= XOR_BYTE);
            }
            self.pos += count as u64;
            self.buf.clear();
            self.buf_pos = 0;
            return Ok(count);
        }
        if self.buf_pos >= self.buf.len() {
            self.fill()?;
            if self.buf.is_empty() {
                return Ok(0);
            }
        }
        let count = out.len().min(self.buf.len() - self.buf_pos);
        out[..count].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + count]);
        self.buf_pos += count;
        self.pos += count as u64;
        Ok(count)
    }
}

impl Seek for ArchiveFile {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => self.len as i128 + offset as i128,
            SeekFrom::Current(offset) => self.pos as i128 + offset as i128,
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the archive",
            ));
        }
        let target = target as u64;
        // Keep the window when the target falls inside it.
        let window_start = self.pos - self.buf_pos as u64;
        let window_end = window_start + self.buf.len() as u64;
        if (window_start..window_end).contains(&target) {
            self.buf_pos = (target - window_start) as usize;
        } else {
            self.buf.clear();
            self.buf_pos = 0;
        }
        self.pos = target;
        Ok(target)
    }
}

/// An opened archive. The central directory is parsed once in
/// [`Archive::open`]; reads clone the parsed index (an `Arc`) with a fresh
/// cursor, so they run in parallel without locking.
pub struct Archive {
    zip: ZipArchive<ArchiveFile>,
    password: Option<Vec<u8>>,
    /// Every directory implied by an entry path, without trailing `/`
    /// (`""` is the root).
    dirs: BTreeSet<String>,
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("entries", &self.zip.len())
            .finish_non_exhaustive()
    }
}

impl Archive {
    /// Opens the archive file at `path`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(path: impl AsRef<Path>, options: &ArchiveOptions) -> io::Result<Self> {
        let file = std::fs::File::open(path.as_ref())?;
        Self::from_backing(Backing::File(file), options)
    }

    /// Opens an archive held in memory (web, Android assets, tests).
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>, options: &ArchiveOptions) -> io::Result<Self> {
        Self::from_backing(Backing::Bytes(bytes.into()), options)
    }

    fn from_backing(backing: Backing, options: &ArchiveOptions) -> io::Result<Self> {
        let file = ArchiveFile::new(backing, options.obfuscate)?;
        let zip = ZipArchive::new(file).map_err(zip_error)?;
        let mut dirs = BTreeSet::new();
        dirs.insert(String::new());
        for name in zip.file_names() {
            let name = name.trim_end_matches('/');
            // Every ancestor of the entry is a directory; explicit
            // directory entries are added below.
            let mut parts: Vec<&str> = name.split('/').collect();
            parts.pop();
            let mut dir = String::new();
            for part in parts {
                if !dir.is_empty() {
                    dir.push('/');
                }
                dir.push_str(part);
                dirs.insert(dir.clone());
            }
        }
        // Explicit directory entries (`name/`) are directories too.
        for name in zip.file_names().filter(|name| name.ends_with('/')) {
            dirs.insert(name.trim_end_matches('/').to_string());
        }
        Ok(Self {
            zip,
            password: options.password.as_ref().map(|p| p.as_bytes().to_vec()),
            dirs,
        })
    }

    /// Number of entries (files and explicit directories).
    pub fn len(&self) -> usize {
        self.zip.len()
    }

    pub fn is_empty(&self) -> bool {
        self.zip.is_empty()
    }

    /// Whether `path` is a file in the archive.
    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        let name = normalize(path.as_ref());
        !name.is_empty() && self.zip.index_for_name(&name).is_some()
    }

    /// Whether `path` is a directory (explicit, or implied by an entry).
    pub fn is_dir(&self, path: impl AsRef<Path>) -> bool {
        self.dirs.contains(&normalize(path.as_ref()))
    }

    /// Reads the file at `path`.
    pub fn read(&self, path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let name = normalize(path.as_ref());
        let Some(index) = self.zip.index_for_name(&name).filter(|_| !name.is_empty()) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{name} is not in the archive"),
            ));
        };
        let mut zip = self.zip.clone();
        let mut file = match &self.password {
            Some(password) => zip.by_index_decrypt(index, password),
            None => zip.by_index(index),
        }
        .map_err(zip_error)?;
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Reads the file at `path` as UTF-8.
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        String::from_utf8(self.read(path)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }

    /// The direct children of `dir` (files and directories), as full
    /// archive paths, sorted.
    pub fn read_dir(&self, dir: impl AsRef<Path>) -> Vec<String> {
        let dir = normalize(dir.as_ref());
        let mut children = BTreeSet::new();
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        for name in self.zip.file_names() {
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let rest = rest.trim_end_matches('/');
            if rest.is_empty() {
                continue;
            }
            let child = rest.split('/').next().unwrap_or(rest);
            children.insert(format!("{prefix}{child}"));
        }
        children.into_iter().collect()
    }

    /// Every file under `dir` (recursively), as full archive paths, sorted.
    pub fn files_under(&self, dir: impl AsRef<Path>) -> Vec<String> {
        let dir = normalize(dir.as_ref());
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut files: Vec<String> = self
            .zip
            .file_names()
            .filter(|name| !name.ends_with('/') && name.starts_with(&prefix))
            .map(str::to_string)
            .collect();
        files.sort();
        files
    }

    /// Every file path in the archive, sorted.
    pub fn file_names(&self) -> Vec<String> {
        self.files_under("")
    }
}

/// An archive entry name for `path`: `/` separators, no leading `./` or
/// `/`, no trailing `/`.
pub fn normalize(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut path = path.as_str();
    loop {
        if let Some(rest) = path.strip_prefix("./") {
            path = rest;
        } else if let Some(rest) = path.strip_prefix('/') {
            path = rest;
        } else {
            break;
        }
    }
    path.trim_end_matches('/').to_string()
}

fn zip_error(err: zip::result::ZipError) -> io::Error {
    match err {
        zip::result::ZipError::Io(err) => err,
        zip::result::ZipError::FileNotFound => io::Error::new(io::ErrorKind::NotFound, err),
        other => io::Error::new(io::ErrorKind::InvalidData, other),
    }
}
