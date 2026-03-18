use bevy_app::Plugin;
use bevy_asset::{
    AssetApp,
    io::{AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream, Reader},
};
use bevy_derive::Deref;
use bevy_reflect::TypePath;
use std::{
    io::{BufReader, Read, Seek},
    path::{Path, PathBuf},
    str::FromStr,
};
use xorio::Xor;
use zip::ZipArchive;

#[cfg(feature = "bundler")]
mod bundler;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum ArchiveCompression {
    #[default]
    None,
    Xz,
}

impl From<ArchiveCompression> for zip::CompressionMethod {
    fn from(compression: ArchiveCompression) -> Self {
        match compression {
            ArchiveCompression::None => zip::CompressionMethod::Stored,
            ArchiveCompression::Xz => zip::CompressionMethod::Xz,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArchivePath {
    FileRelativeToApplicationDirectory(String),
    AbsolutePath(PathBuf),
}

impl Default for ArchivePath {
    fn default() -> Self {
        Self::FileRelativeToApplicationDirectory("file.zip".to_string())
    }
}

impl From<&ArchivePath> for PathBuf {
    fn from(value: &ArchivePath) -> Self {
        match value {
            ArchivePath::FileRelativeToApplicationDirectory(file_name) => {
                if cfg!(target_os = "android") {
                    return PathBuf::from_str(file_name.as_str()).unwrap_or_default();
                }
                let Ok(exe_dir) = std::env::current_exe() else {
                    return PathBuf::from_str(file_name.as_str()).unwrap_or_default();
                };
                let Some(exe_dir) = exe_dir.parent() else {
                    return PathBuf::from_str(file_name.as_str()).unwrap_or_default();
                };
                if cfg!(target_os = "ios") {
                    // iOS .app bundles have the executable at the bundle root
                    // Resources are also at the bundle root
                    exe_dir.join(file_name)
                } else {
                    // macOS .app bundles have exe at Contents/MacOS/binary
                    // Resources at Contents/Resources/
                    exe_dir.join("..").join(file_name)
                }
            }
            ArchivePath::AbsolutePath(path_buf) => path_buf.clone(),
        }
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
}

#[derive(Deref, TypePath)]
pub struct ArchiveAssetReader {
    #[deref]
    settings: ArchiveReaderSettings,
}
impl ArchiveAssetReader {
    pub fn new(settings: ArchiveReaderSettings) -> Self {
        Self { settings }
    }
}
#[derive(Clone, Debug, Default)]
pub struct ArchivePlugin {
    settings: ArchiveReaderSettings,
}
impl ArchivePlugin {
    pub fn with_path_relative(self, path: impl ToString) -> Self {
        Self {
            settings: self.settings.with_path_relative(path),
        }
    }
    pub fn with_path(self, path: ArchivePath) -> Self {
        Self {
            settings: self.settings.with_path(path),
        }
    }
    pub fn with_obfuscate(self, obfuscate: bool) -> Self {
        Self {
            settings: self.settings.with_obfuscate(obfuscate),
        }
    }
    pub fn with_password(self, password: impl ToString) -> Self {
        Self {
            settings: self.settings.with_password(password),
        }
    }
}

impl Plugin for ArchivePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        let settings = self.settings.clone();

        let builder =
            AssetSourceBuilder::new(move || Box::new(ArchiveAssetReader::new(settings.clone())));
        app.register_asset_source(AssetSourceId::Default, builder);
    }
}
trait FileReader: Read + Seek + Sync + Send {}
impl<T: Read + Seek + Sync + Send> FileReader for T {}

impl ArchiveAssetReader {
    fn get_reader(&self) -> Option<ZipArchive<Box<dyn FileReader>>> {
        let path = self.path.get_zip_path();
        #[cfg(target_os = "android")]
        let file = {
            use std::ffi::CString;
            use std::io::Cursor;
            let asset_manager = bevy_android::ANDROID_APP
                .get()
                .expect("Bevy must be setup with the #[bevy_main] macro on Android")
                .asset_manager();
            let mut opened_asset =
                asset_manager.open(&CString::new(path.to_str().unwrap()).unwrap())?;
            let bytes = opened_asset.buffer().ok()?.to_vec();
            Cursor::new(bytes)
        };
        #[cfg(not(target_os = "android"))]
        let file = std::fs::OpenOptions::new().read(true).open(path).ok()?;
        let reader: Box<dyn FileReader> = if self.obfuscate {
            Box::new(Xor::new(file))
        } else {
            Box::new(file)
        };

        ZipArchive::new(Box::new(BufReader::new(reader)) as Box<dyn FileReader>).ok()
    }
    pub async fn read_file<'a>(
        &'a self,
        path: &'a std::path::Path,
        is_meta: bool,
    ) -> Result<impl Reader + 'a, AssetReaderError> {
        let Some(mut archive) = self.get_reader() else {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        };
        let path = if is_meta {
            path.with_added_extension("meta")
        } else {
            path.to_path_buf()
        };
        let file = match &self.password {
            Some(password) => {
                archive.by_name_decrypt(path.to_str().expect("msg"), password.as_bytes())
            }
            None => archive.by_name(path.to_str().expect("msg")),
        };
        let buf = if let Ok(mut file) = file {
            let size = file.size();

            let mut buf = Vec::with_capacity(size as usize);
            file.read_to_end(&mut buf).expect("msg");
            buf
        } else {
            bevy_log::error!("There is no file in the archive: {}", path.display());
            return Err(AssetReaderError::NotFound(path));
        };
        let reader: Box<dyn bevy_asset::io::Reader> = Box::new(bevy_asset::io::VecReader::new(buf));
        Ok(reader)
    }
}

impl Default for ArchiveAssetReader {
    fn default() -> Self {
        Self::new(ArchiveReaderSettings {
            obfuscate: false,
            path: ArchivePath::default(),
            password: None,
        })
    }
}
impl AssetReader for ArchiveAssetReader {
    async fn read<'a>(
        &'a self,
        path: &'a std::path::Path,
    ) -> Result<impl Reader + 'a, AssetReaderError> {
        self.read_file(path, false).await
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.read_file(path, true).await
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let Some(mut archive) = self.get_reader() else {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        };
        let mut mapped_stream = Vec::new();
        for i in 0..archive.len() {
            let Ok(file) = archive.by_index(i) else {
                continue;
            };
            let el_path = file.mangled_name();
            if el_path.parent().is_some_and(|p| p == path) {
                mapped_stream.push(el_path);
            }
        }
        let read_dir: Box<PathStream> = Box::new(futures_lite::stream::iter(mapped_stream));
        Ok(read_dir)
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        if let Some(mut archive) = self.get_reader() {
            archive
                .by_name(path.to_str().expect("msg"))
                .map(|f| f.is_dir())
                .map_err(|_| AssetReaderError::NotFound(path.to_path_buf()))
        } else {
            Err(AssetReaderError::NotFound(path.to_path_buf()))
        }
    }
}

#[cfg(feature = "bundler")]
pub fn bundle_assets(source_path: impl AsRef<Path>, settings: ArchiveSaveSettings) {
    let cargo_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let asset_dir = std::path::PathBuf::from(&cargo_dir).join(source_path);
    let archive_path = match settings.path {
        ArchivePath::FileRelativeToApplicationDirectory(rel) => {
            std::path::PathBuf::from(cargo_dir).join(rel)
        }
        ArchivePath::AbsolutePath(path_buf) => path_buf,
    };
    bundler::zip_dir(
        &asset_dir,
        &archive_path,
        settings.compression.into(),
        settings.password,
        settings.obfuscate,
    );
}
