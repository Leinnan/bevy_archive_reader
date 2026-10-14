# Changelog

## [0.4.0]

### Added

- `Archive`: opens an archive once (central directory parsed once) and reads
  synchronously from any thread (`read`, `read_to_string`, `exists`, `is_dir`,
  `read_dir`, `files_under`). Readers clone the parsed index with their own
  cursor, so parallel reads don't lock or re-parse.
- `ArchivePlugin::from_archive`, `with_source` and `with_prefix`: one opened
  archive can back several asset sources, each rooted at a directory.
- `ArchivePath::BundleResource` (where `cargo-bundler` puts a resource on each
  platform) and `ArchivePath::NextToExecutable`; `bundle_resource_path`.
- `ArchiveCompression::Deflate` and `ArchiveCompression::Auto` (stores
  already compressed files such as `ktx2`/`ogg`/`png`/`glb`, deflates the rest).
- `bundle_dir`: packs a directory with an include filter, sorted and with
  fixed timestamps, so the output is reproducible.

### Changed

- The `bundler` feature is no longer on by default.
- Missing files (most `.meta` lookups) are reported as `NotFound` without
  logging an error.
- `is_directory` and `read_directory` work for directories implied by file
  paths, not only explicit directory entries.
- `bundle_assets` no longer panics outside cargo (`CARGO_MANIFEST_DIR` unset)
  and accepts absolute paths.
- XOR obfuscation is implemented in-crate (the `xorio` dependency is gone);
  archives obfuscated by 0.3 stay readable.

## [0.3.0]

### Changed

- Updated to Bevy 0.19.0

## [0.2.0]

### Changed

- Updated to Bevy 0.18.0

## [0.1.0]

### Added

- Initial version
