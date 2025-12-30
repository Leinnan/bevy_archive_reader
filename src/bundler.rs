#![allow(unused_variables)]
#![allow(dead_code)]
use std::{
    fs::File,
    io::{BufWriter, Read, Seek, Write},
    path::Path,
};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

trait WriteSeek: Seek + Write + Read + Send {}
impl<T: Seek + Write + Send + Read> WriteSeek for T {}

pub(crate) fn zip_dir(
    src_dir: &Path,
    dst_file: &Path,
    method: zip::CompressionMethod,
    password: Option<String>,
    obfuscate: bool,
) {
    use std::time::Instant;
    let now = Instant::now();
    let path = Path::new(dst_file);
    let archive_file = File::create(dst_file).expect("Could not create archive file");
    let writer: Box<dyn WriteSeek> = if obfuscate {
        Box::new(xorio::Xor::new(archive_file))
    } else {
        Box::new(archive_file)
    };
    let writer = BufWriter::new(writer);

    let prefix = src_dir;
    let walkdir = WalkDir::new(prefix);

    use bevy_tasks::TaskPool;
    let task_pool = TaskPool::new();
    let mut files = Vec::new();
    let mut directories = Vec::new();

    for entry in WalkDir::new(prefix).into_iter().flatten() {
        let path = entry.path();
        if path.is_file() {
            files.push(entry);
        } else if path.is_dir()
            && let Ok(name) = path.strip_prefix(prefix)
            && !name.as_os_str().is_empty()
        {
            directories.push(name.to_string_lossy().into_owned());
        }
    }

    bevy_log::info!(
        "Found {} files and {} directories",
        files.len(),
        directories.len()
    );
    let mut zip = zip::ZipWriter::new(writer);
    let mut options = SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(0o755);
    if let Some(pwd) = &password {
        options = options.with_aes_encryption(zip::AesMode::Aes256, pwd.as_str());
    }
    for dir in directories {
        zip.add_directory(dir, options)
            .expect("Failed to add directory to zip");
    }
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let chunk_size = (files.len() / task_pool.thread_num()).clamp(10, 100);

    // Process files in parallel - only read and prepare data
    let file_count = files.len();
    task_pool.scope(|scope| {
        for chunk in files.chunks(chunk_size) {
            let tx = tx.clone();
            let prefix = prefix.to_path_buf();
            let chunk = chunk.to_vec();

            scope.spawn(async move {
                for entry in chunk {
                    let path = entry.path();
                    let Ok(name) = path
                        .strip_prefix(&prefix)
                        .map(|p| p.to_string_lossy().into_owned())
                    else {
                        bevy_log::error!("Failed to strip path prefix from {:?}", path);
                        continue;
                    };

                    // Read file into memory
                    match std::fs::read(path) {
                        Ok(data) => {
                            tx.send((name, data)).ok();
                        }
                        Err(e) => {
                            bevy_log::error!("Failed to read {:?}: {}", path, e);
                        }
                    }
                }
            });
        }
    });

    // Drop the original sender so receiver knows when done
    drop(tx);

    // Write files sequentially to zip (compression happens here)
    let mut processed = 0;
    while let Ok((name, data)) = rx.recv() {
        zip.start_file(&name, options)
            .expect("Failed to start file");
        zip.write_all(&data)
            .expect("Failed to write file data to zip");
        processed += 1;
        if processed % 15 == 0 {
            bevy_log::info!(
                "Processed {}/{} files, time: {}ms",
                processed,
                file_count,
                now.elapsed().as_millis()
            );
        }
    }
    zip.finish().expect("Failed to finish zip");
    let elapsed = now.elapsed();
    bevy_log::info!("Finished bundling in {}ms", elapsed.as_millis());
}
