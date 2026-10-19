//! Bevy assets from a zip archive (optionally obfuscated, encrypted and
//! compressed). With default features off, only [`Archive`] (reading) and,
//! with `bundler`, [`bundle_dir`] (writing) remain: no Bevy dependency.

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

mod archive;
#[cfg(feature = "bundler")]
mod bundler;
#[cfg(feature = "bevy")]
mod plugin;

pub use archive::{Archive, ArchiveOptions, normalize};
#[cfg(feature = "bundler")]
pub use bundler::{bundle_dir, compression_for};
#[cfg(feature = "bevy")]
pub use plugin::{ArchiveAssetReader, ArchivePlugin};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArchiveCompression {
    /// Store every file as is.
    #[default]
    None,
    /// Deflate every file.
    Deflate,
    /// Xz every file: smallest, but slow to decompress.
    Xz,
    /// Store files that are already compressed (textures, audio, images,
    /// glTF binaries) and deflate the rest.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArchivePath {
    /// `<exe dir>/../<file>` on desktop (so `Contents/<file>` in a macOS
    /// bundle), `<exe dir>/<file>` on iOS, the asset name on Android. Kept
    /// for 0.3 compatibility; prefer [`ArchivePath::BundleResource`].
    FileRelativeToApplicationDirectory(String),
    /// `<exe dir>/<file>`: portable folders, Steam depots.
    NextToExecutable(String),
    /// Where `cargo-bundler` puts a resource mapped to the bundle root:
    /// - macOS: `<App>.app/Contents/Resources/<file>`
    /// - iOS: `<App>.app/<file>`
    /// - Linux (AppImage, deb): `usr/lib/<binary>/<file>`
    /// - Windows (msi): `<install dir>/Resources/<file>`
    /// - Android: the asset name.
    BundleResource(String),
    AbsolutePath(PathBuf),
}

impl Default for ArchivePath {
    fn default() -> Self {
        Self::FileRelativeToApplicationDirectory("file.zip".to_string())
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

impl From<&ArchivePath> for PathBuf {
    fn from(value: &ArchivePath) -> Self {
        let plain = |name: &str| PathBuf::from_str(name).unwrap_or_default();
        match value {
            ArchivePath::FileRelativeToApplicationDirectory(file_name) => {
                if cfg!(target_os = "android") {
                    return plain(file_name);
                }
                let Some(exe_dir) = exe_dir() else {
                    return plain(file_name);
                };
                if cfg!(target_os = "ios") {
                    exe_dir.join(file_name)
                } else {
                    exe_dir.join("..").join(file_name)
                }
            }
            ArchivePath::NextToExecutable(file_name) => {
                exe_dir().map_or_else(|| plain(file_name), |dir| dir.join(file_name))
            }
            ArchivePath::BundleResource(file_name) => bundle_resource_path(file_name),
            ArchivePath::AbsolutePath(path_buf) => path_buf.clone(),
        }
    }
}

/// Where `cargo-bundler` installs the resource `file_name` (mapped to the
/// bundle root) for the running executable. See
/// [`ArchivePath::BundleResource`].
pub fn bundle_resource_path(file_name: &str) -> PathBuf {
    let plain = || PathBuf::from_str(file_name).unwrap_or_default();
    if cfg!(target_os = "android") {
        return plain();
    }
    let Ok(exe) = std::env::current_exe() else {
        return plain();
    };
    let Some(exe_dir) = exe.parent() else {
        return plain();
    };
    if cfg!(target_os = "ios") {
        exe_dir.join(file_name)
    } else if cfg!(target_os = "macos") {
        exe_dir.join("../Resources").join(file_name)
    } else if cfg!(target_os = "windows") {
        exe_dir.join("Resources").join(file_name)
    } else {
        let binary = exe
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        exe_dir.join("../lib").join(binary).join(file_name)
    }
}

impl ArchivePath {
    #[inline]
    pub fn get_zip_path(&self) -> PathBuf {
        self.into()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveSaveSettings {
    pub obfuscate: bool,
    pub path: ArchivePath,
    pub password: Option<String>,
    pub compression: ArchiveCompression,
}

impl ArchiveSaveSettings {
    pub fn with_path_relative(self, path: impl ToString) -> Self {
        Self {
            path: ArchivePath::FileRelativeToApplicationDirectory(path.to_string()),
            ..self
        }
    }
    pub fn with_path(self, path: ArchivePath) -> Self {
        Self { path, ..self }
    }
    pub fn with_obfuscate(self, obfuscate: bool) -> Self {
        Self { obfuscate, ..self }
    }
    pub fn with_password(self, password: impl ToString) -> Self {
        Self {
            password: Some(password.to_string()),
            ..self
        }
    }
    pub fn with_compression(self, compression: ArchiveCompression) -> Self {
        Self {
            compression,
            ..self
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveReaderSettings {
    pub obfuscate: bool,
    pub path: ArchivePath,
    pub password: Option<String>,
}

impl ArchiveReaderSettings {
    pub fn with_path_relative(self, path: impl ToString) -> Self {
        Self {
            path: ArchivePath::FileRelativeToApplicationDirectory(path.to_string()),
            ..self
        }
    }
    pub fn with_path(self, path: ArchivePath) -> Self {
        Self { path, ..self }
    }
    pub fn with_obfuscate(self, obfuscate: bool) -> Self {
        Self { obfuscate, ..self }
    }
    pub fn with_password(self, password: impl ToString) -> Self {
        Self {
            password: Some(password.to_string()),
            ..self
        }
    }
    pub fn options(&self) -> ArchiveOptions {
        ArchiveOptions {
            obfuscate: self.obfuscate,
            password: self.password.clone(),
        }
    }
}

/// Packs `source_path` into an archive. Relative paths resolve against
/// `CARGO_MANIFEST_DIR` (or the current directory outside cargo);
/// absolute paths are used as is. See [`bundle_dir`] for filtering.
#[cfg(feature = "bundler")]
pub fn bundle_assets(source_path: impl AsRef<Path>, settings: ArchiveSaveSettings) {
    let base = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let asset_dir = base.join(source_path);
    let archive_path = match &settings.path {
        ArchivePath::FileRelativeToApplicationDirectory(rel)
        | ArchivePath::NextToExecutable(rel)
        | ArchivePath::BundleResource(rel) => base.join(rel),
        ArchivePath::AbsolutePath(path_buf) => path_buf.clone(),
    };
    if let Err(err) = bundle_dir(&asset_dir, &archive_path, &settings, |_| true) {
        panic!("Failed to bundle {}: {err}", asset_dir.display());
    }
}
