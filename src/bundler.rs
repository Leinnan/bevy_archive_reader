//! Writing archives (`bundler` feature). Keep this out of shipped games.

use std::{
    fs::File,
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::Path,
};
use walkdir::WalkDir;
use zip::{CompressionMethod, write::SimpleFileOptions};

use crate::{ArchiveCompression, ArchiveSaveSettings, archive::XOR_BYTE};

/// Extensions [`ArchiveCompression::Auto`] stores uncompressed: already
/// compressed, or not worth the decompression time.
const STORED_EXTENSIONS: &[&str] = &[
    "ktx2", "basis", "dds", "png", "jpg", "jpeg", "webp", "ogg", "mp3", "flac", "opus", "glb",
    "zip", "zst", "gz", "xz",
];

/// The compression method `compression` picks for the file at `path`.
pub fn compression_for(compression: ArchiveCompression, path: &Path) -> CompressionMethod {
    match compression {
        ArchiveCompression::None => CompressionMethod::Stored,
        ArchiveCompression::Deflate => CompressionMethod::Deflated,
        ArchiveCompression::Xz => CompressionMethod::Xz,
        ArchiveCompression::Auto => {
            let stored = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    STORED_EXTENSIONS
                        .iter()
                        .any(|stored| ext.eq_ignore_ascii_case(stored))
                });
            if stored {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            }
        }
    }
}

/// XORs every written byte with [`XOR_BYTE`].
struct XorWriter<W> {
    inner: W,
    scratch: Vec<u8>,
}

impl<W: Write> Write for XorWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.scratch.clear();
        self.scratch.extend(buf.iter().map(|byte| byte ^ XOR_BYTE));
        self.inner.write_all(&self.scratch)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Seek> Seek for XorWriter<W> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

trait WriteSeek: Write + Seek {}
impl<T: Write + Seek> WriteSeek for T {}

/// Packs every file under `src_dir` for which `include` (given the path
/// relative to `src_dir`) returns true into `dst_file`. Entries are
/// written in sorted order with fixed timestamps, so the same input gives
/// a byte-identical archive (unless a password is set: AES salts are
/// random). Returns the number of files written.
pub fn bundle_dir(
    src_dir: &Path,
    dst_file: &Path,
    settings: &ArchiveSaveSettings,
    include: impl Fn(&Path) -> bool,
) -> io::Result<usize> {
    let started = std::time::Instant::now();
    let mut files = Vec::new();
    for entry in WalkDir::new(src_dir).follow_links(true) {
        let entry = entry.map_err(io::Error::other)?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(src_dir)
            .map_err(io::Error::other)?
            .to_path_buf();
        if include(&relative) {
            files.push((crate::normalize(&relative), entry.path().to_path_buf()));
        }
    }
    files.sort();

    if let Some(parent) = dst_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let archive_file = File::create(dst_file)?;
    let writer: Box<dyn WriteSeek> = if settings.obfuscate {
        Box::new(XorWriter {
            inner: archive_file,
            scratch: Vec::new(),
        })
    } else {
        Box::new(archive_file)
    };
    let mut zip = zip::ZipWriter::new(BufWriter::with_capacity(1 << 20, writer));
    for (name, path) in &files {
        let mut options = SimpleFileOptions::default()
            .compression_method(compression_for(settings.compression, path))
            .unix_permissions(0o644)
            .large_file(std::fs::metadata(path)?.len() >= u32::MAX as u64);
        if let Some(password) = &settings.password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, password.as_str());
        }
        zip.start_file(name.as_str(), options)
            .map_err(io::Error::other)?;
        io::copy(&mut File::open(path)?, &mut zip)?;
    }
    zip.finish().map_err(io::Error::other)?.flush()?;
    bevy_log::info!(
        "Bundled {} files into {} in {}ms",
        files.len(),
        dst_file.display(),
        started.elapsed().as_millis()
    );
    Ok(files.len())
}
