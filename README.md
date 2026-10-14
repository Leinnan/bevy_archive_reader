# Bevy Archive Reader

[![Crates.io](https://img.shields.io/crates/v/bevy_archive_reader)](https://crates.io/crates/bevy_archive_reader)
[![Documentation](https://docs.rs/bevy_archive_reader/badge.svg)](https://docs.rs/bevy_archive_reader)
[![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/bevy_archive_reader/bevy_archive_reader#license)

Simple crate for reading Bevy assets from archives. Supports adding password protection, obfuscation and compression. Support for generating archives is included behind the `bundler` feature.

### Using in game

```rust
    app.add_plugins(
        bevy_archive_reader::ArchivePlugin::default()
            .with_path_relative("file.zip")
            .with_password("SomeSecretPassword"),
    );
```

One opened archive can back several sources, and game code can read from it
directly (for example to parse config files outside the `AssetServer`):

```rust
use std::sync::Arc;
use bevy_archive_reader::{Archive, ArchiveOptions, ArchivePath, ArchivePlugin};

let path = ArchivePath::BundleResource("game.pak".into()).get_zip_path();
let archive = Arc::new(Archive::open(path, &ArchiveOptions::default().with_obfuscate(true))?);
let config = archive.read_to_string("config/settings.toml")?;

app.add_plugins((
    ArchivePlugin::from_archive(archive.clone()),
    ArchivePlugin::from_archive(archive).with_source("models").with_prefix("models"),
));
```

`ArchivePath::BundleResource` resolves where `cargo-bundler` installs a
resource mapped to the bundle root: `Contents/Resources/` on macOS,
`usr/lib/<binary>/` in an AppImage or deb, `<install dir>/Resources/` for an
MSI.

### Generating Archives

Generating archives needs the `bundler` feature (off by default) and should be done outside of the shipped game, for example in a build script or a separate tool binary. `bundle_dir` takes an include filter and writes a reproducible archive; `ArchiveCompression::Auto` stores already compressed files and deflates the rest.

```rust
        bevy_archive_reader::bundle_assets(
            "imported_assets/Default", // or "assets", depends on the use case
            ArchiveSaveSettings::default()
                .with_password("SomeSecretPassword")
                .with_compression(bevy_archive_reader::ArchiveCompression::Xz)
                .with_path_relative("file.zip"),
```

## License

`bevy_archive_reader` is dual-licensed under MIT and Apache 2.0 at your option.

## Compatibility

| bevy | bevy_archive_reader |
| ---: | ---------: |
| 0.19 |   0.3, 0.4 |
| 0.18 |        0.2 |
| 0.17 |        0.1 |
