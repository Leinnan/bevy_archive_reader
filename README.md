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

### Generating Archives

Generating archives should be done outside of the game (or at least outside of the shipped game), for example in a build script.

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
| 0.18 |        0.2 |
| 0.17 |        0.1 |
