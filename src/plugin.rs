//! The Bevy side: [`ArchiveAssetReader`] and [`ArchivePlugin`].

use bevy_app::Plugin;
use bevy_asset::{
    AssetApp,
    io::{AssetReader, AssetReaderError, AssetSourceBuilder, AssetSourceId, PathStream, Reader},
};
use bevy_reflect::TypePath;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{Archive, ArchiveOptions, ArchivePath, ArchiveReaderSettings, normalize};

/// Where an [`ArchiveAssetReader`] gets its archive.
#[derive(Clone)]
enum ArchiveSource {
    /// Opened on first use from the settings, and shared by every reader
    /// the plugin's source builder creates.
    Lazy(
        ArchiveReaderSettings,
        Arc<async_lock::OnceCell<Option<Arc<Archive>>>>,
    ),
    /// Opened by the caller.
    Ready(Arc<Archive>),
}

/// An [`AssetReader`] over an [`Archive`], optionally rooted at a
/// directory inside it.
#[derive(TypePath)]
pub struct ArchiveAssetReader {
    source: ArchiveSource,
    prefix: String,
}

impl ArchiveAssetReader {
    /// A reader that opens the archive described by `settings` on first
    /// use.
    pub fn new(settings: ArchiveReaderSettings) -> Self {
        Self {
            source: ArchiveSource::Lazy(settings, Arc::default()),
            prefix: String::new(),
        }
    }

    /// A reader over an already opened archive.
    pub fn from_archive(archive: Arc<Archive>) -> Self {
        Self {
            source: ArchiveSource::Ready(archive),
            prefix: String::new(),
        }
    }

    /// Roots the reader at `prefix` inside the archive (`models` makes
    /// `a.glb` read `models/a.glb`).
    pub fn with_prefix(self, prefix: impl AsRef<Path>) -> Self {
        Self {
            prefix: normalize(prefix.as_ref()),
            ..self
        }
    }

    async fn archive(&self) -> Option<&Arc<Archive>> {
        match &self.source {
            ArchiveSource::Ready(archive) => Some(archive),
            ArchiveSource::Lazy(settings, cell) => {
                cell.get_or_init(|| open_archive(settings)).await.as_ref()
            }
        }
    }

    fn full_path(&self, path: &Path) -> String {
        let path = normalize(path);
        match (self.prefix.is_empty(), path.is_empty()) {
            (true, _) => path,
            (false, true) => self.prefix.clone(),
            (false, false) => format!("{}/{path}", self.prefix),
        }
    }

    fn relative_path(&self, full: &str) -> PathBuf {
        if self.prefix.is_empty() {
            return PathBuf::from(full);
        }
        PathBuf::from(
            full.strip_prefix(&self.prefix)
                .map(|rest| rest.trim_start_matches('/'))
                .unwrap_or(full),
        )
    }

    pub async fn read_file<'a>(
        &'a self,
        path: &'a Path,
        is_meta: bool,
    ) -> Result<impl Reader + 'a, AssetReaderError> {
        let not_found = || {
            AssetReaderError::NotFound(if is_meta {
                path.with_added_extension("meta")
            } else {
                path.to_path_buf()
            })
        };
        let Some(archive) = self.archive().await else {
            return Err(not_found());
        };
        let mut full = self.full_path(path);
        if is_meta {
            full.push_str(".meta");
        }
        // Missing files (most `.meta` lookups) are expected: report them
        // without logging.
        if !archive.exists(&full) {
            return Err(not_found());
        }
        let bytes = archive.read(&full).map_err(|err| {
            bevy_log::error!("Failed to read {full} from the archive: {err}");
            AssetReaderError::Io(Arc::new(err))
        })?;
        let reader: Box<dyn Reader> = Box::new(bevy_asset::io::VecReader::new(bytes));
        Ok(reader)
    }
}

impl Default for ArchiveAssetReader {
    fn default() -> Self {
        Self::new(ArchiveReaderSettings::default())
    }
}

/// Opens the archive `settings` describes, logging failures.
async fn open_archive(settings: &ArchiveReaderSettings) -> Option<Arc<Archive>> {
    let path = settings.path.get_zip_path();
    match open_platform(&path, &settings.options()).await {
        Ok(archive) => Some(Arc::new(archive)),
        Err(err) => {
            bevy_log::error!("Failed to open archive {}: {err}", path.display());
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_platform(path: &Path, options: &ArchiveOptions) -> std::io::Result<Archive> {
    use bevy_asset::io::wasm::HttpWasmAssetReader;

    let http = HttpWasmAssetReader::new("");
    let mut reader = http
        .read(path)
        .await
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    Archive::from_bytes(bytes, options)
}

#[cfg(target_os = "android")]
async fn open_platform(path: &Path, options: &ArchiveOptions) -> std::io::Result<Archive> {
    use std::ffi::CString;
    let asset_manager = bevy_android::ANDROID_APP
        .get()
        .expect("Bevy must be setup with the #[bevy_main] macro on Android")
        .asset_manager();
    let name = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let mut asset = asset_manager
        .open(&name)
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let bytes = asset.buffer()?.to_vec();
    Archive::from_bytes(bytes, options)
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
async fn open_platform(path: &Path, options: &ArchiveOptions) -> std::io::Result<Archive> {
    Archive::open(path, options)
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
        let Some(archive) = self.archive().await else {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        };
        let full = self.full_path(path);
        if !archive.is_dir(&full) {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        }
        let children: Vec<PathBuf> = archive
            .read_dir(&full)
            .iter()
            .map(|child| self.relative_path(child))
            .collect();
        let read_dir: Box<PathStream> = Box::new(futures_lite::stream::iter(children));
        Ok(read_dir)
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        let Some(archive) = self.archive().await else {
            return Err(AssetReaderError::NotFound(path.to_path_buf()));
        };
        let full = self.full_path(path);
        if archive.is_dir(&full) {
            Ok(true)
        } else if archive.exists(&full) {
            Ok(false)
        } else {
            Err(AssetReaderError::NotFound(path.to_path_buf()))
        }
    }
}

/// Registers an archive as an asset source (the default source unless
/// [`ArchivePlugin::with_source`] says otherwise). Add it before
/// `AssetPlugin` when replacing the default source.
#[derive(Clone)]
pub struct ArchivePlugin {
    settings: ArchiveReaderSettings,
    archive: Option<Arc<Archive>>,
    source: AssetSourceId<'static>,
    prefix: String,
}

impl Default for ArchivePlugin {
    fn default() -> Self {
        Self {
            settings: ArchiveReaderSettings::default(),
            archive: None,
            source: AssetSourceId::Default,
            prefix: String::new(),
        }
    }
}

impl ArchivePlugin {
    /// Serves an archive the caller already opened, so one [`Archive`] can
    /// back several sources.
    pub fn from_archive(archive: Arc<Archive>) -> Self {
        Self {
            archive: Some(archive),
            ..Self::default()
        }
    }
    pub fn with_path_relative(self, path: impl ToString) -> Self {
        Self {
            settings: self.settings.with_path_relative(path),
            ..self
        }
    }
    pub fn with_path(self, path: ArchivePath) -> Self {
        Self {
            settings: self.settings.with_path(path),
            ..self
        }
    }
    pub fn with_obfuscate(self, obfuscate: bool) -> Self {
        Self {
            settings: self.settings.with_obfuscate(obfuscate),
            ..self
        }
    }
    pub fn with_password(self, password: impl ToString) -> Self {
        Self {
            settings: self.settings.with_password(password),
            ..self
        }
    }
    /// The asset source to register (default: [`AssetSourceId::Default`]).
    pub fn with_source(self, source: impl Into<AssetSourceId<'static>>) -> Self {
        Self {
            source: source.into(),
            ..self
        }
    }
    /// Roots the source at a directory inside the archive.
    pub fn with_prefix(self, prefix: impl AsRef<Path>) -> Self {
        Self {
            prefix: normalize(prefix.as_ref()),
            ..self
        }
    }

    /// The source builder this plugin registers, for callers that register
    /// sources themselves.
    pub fn source_builder(&self) -> AssetSourceBuilder {
        let source = match &self.archive {
            Some(archive) => ArchiveSource::Ready(archive.clone()),
            None => ArchiveSource::Lazy(self.settings.clone(), Arc::default()),
        };
        let prefix = self.prefix.clone();
        AssetSourceBuilder::new(move || {
            Box::new(ArchiveAssetReader {
                source: source.clone(),
                prefix: prefix.clone(),
            })
        })
    }
}

impl Plugin for ArchivePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.register_asset_source(self.source.clone(), self.source_builder());
    }
}
