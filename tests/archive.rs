//! Round trips through `bundle_dir` and `Archive` (needs `--features bundler`).
#![cfg(feature = "bundler")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy_archive_reader::{
    Archive, ArchiveCompression, ArchiveOptions, ArchiveSaveSettings, bundle_dir, compression_for,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bar-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tree(root: &Path) {
    for (path, body) in [
        ("system/classes.toml", "warrior = 1\n".repeat(50)),
        ("models/a.glb", "glb".into()),
        ("models/deep/b.ktx2", "ktx".into()),
        ("lang/en/menu.txt", "Start".into()),
        ("skip.blend", "blend".into()),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
}

fn build(name: &str, settings: &ArchiveSaveSettings) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let src = dir.join("src");
    tree(&src);
    let pak = dir.join("out/test.pak");
    let count = bundle_dir(&src, &pak, settings, |path| {
        path.extension().is_none_or(|ext| ext != "blend")
    })
    .unwrap();
    assert_eq!(count, 4);
    (src, pak)
}

#[test]
fn round_trip_with_listing() {
    let settings = ArchiveSaveSettings::default().with_compression(ArchiveCompression::Auto);
    let (src, pak) = build("plain", &settings);
    let archive = Archive::open(&pak, &ArchiveOptions::default()).unwrap();

    assert_eq!(
        archive.read_to_string("system/classes.toml").unwrap(),
        std::fs::read_to_string(src.join("system/classes.toml")).unwrap()
    );
    assert_eq!(archive.read("./models/a.glb").unwrap(), b"glb");
    assert_eq!(archive.read("/models\\deep/b.ktx2").unwrap(), b"ktx");
    assert!(archive.read("skip.blend").is_err());
    assert_eq!(
        archive.read("missing.txt").unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );

    assert!(archive.exists("lang/en/menu.txt"));
    assert!(!archive.exists("lang/en"));
    assert!(archive.is_dir("lang/en"));
    assert!(archive.is_dir("lang/en/"));
    assert!(archive.is_dir(""));
    assert!(!archive.is_dir("lang/en/menu.txt"));

    assert_eq!(archive.read_dir("models"), ["models/a.glb", "models/deep"]);
    assert_eq!(
        archive.read_dir(""),
        ["lang", "models", "system"].map(String::from)
    );
    assert_eq!(
        archive.files_under("models"),
        ["models/a.glb", "models/deep/b.ktx2"]
    );
}

#[test]
fn obfuscated_and_encrypted() {
    let settings = ArchiveSaveSettings::default()
        .with_obfuscate(true)
        .with_password("secret")
        .with_compression(ArchiveCompression::Deflate);
    let (_, pak) = build("locked", &settings);

    assert!(Archive::open(&pak, &ArchiveOptions::default()).is_err());
    let options = ArchiveOptions::default()
        .with_obfuscate(true)
        .with_password("secret");
    let archive = Archive::open(&pak, &options).unwrap();
    assert_eq!(archive.read("lang/en/menu.txt").unwrap(), b"Start");

    let wrong = Archive::open(&pak, &ArchiveOptions::default().with_obfuscate(true)).unwrap();
    assert!(wrong.read("lang/en/menu.txt").is_err());
}

#[test]
fn deterministic_output() {
    let settings = ArchiveSaveSettings::default().with_compression(ArchiveCompression::Auto);
    let (_, first) = build("det-a", &settings);
    let (_, second) = build("det-b", &settings);
    assert_eq!(
        std::fs::read(first).unwrap(),
        std::fs::read(second).unwrap()
    );
}

#[test]
fn parallel_reads_share_one_index() {
    let (_, pak) = build("parallel", &ArchiveSaveSettings::default());
    let bytes = std::fs::read(&pak).unwrap();
    let archive = Arc::new(Archive::from_bytes(bytes, &ArchiveOptions::default()).unwrap());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let archive = archive.clone();
            scope.spawn(move || {
                for _ in 0..100 {
                    assert_eq!(archive.read("models/a.glb").unwrap(), b"glb");
                    assert!(archive.read_to_string("system/classes.toml").is_ok());
                }
            });
        }
    });
}

#[test]
fn auto_compression_policy() {
    use zip::CompressionMethod;
    let auto = ArchiveCompression::Auto;
    assert_eq!(
        compression_for(auto, Path::new("a.KTX2")),
        CompressionMethod::Stored
    );
    assert_eq!(
        compression_for(auto, Path::new("a.ogg")),
        CompressionMethod::Stored
    );
    assert_eq!(
        compression_for(auto, Path::new("a.toml")),
        CompressionMethod::Deflated
    );
    assert_eq!(
        compression_for(auto, Path::new("noext")),
        CompressionMethod::Deflated
    );
}
