use super::*;
use crate::reader::{ByteReader, FileReader};

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn fixture_dir(tag: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures").join(format!("linux-readers-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn linux_readers_survive_low_descriptor_limit() {
    let _serial = serial();
    const CHILD: &str = "FILMCRAFT_TEST_LOW_FD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "ulimit -n 64 && exec \"$@\"", "filmcraft-reader-test"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "reader::linux_files::tests::linux_readers_survive_low_descriptor_limit", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success(), "256 media readers must work with a 64-descriptor process limit");
        return;
    }
    let dir = fixture_dir("limit");
    let readers: Vec<_> = (0u32..256)
        .map(|i| {
            let path = dir.join(format!("{i}.bin"));
            let mut data = [0u8; 16];
            data[..4].copy_from_slice(&i.to_le_bytes());
            std::fs::write(&path, data).unwrap();
            FileReader::open(&path).unwrap()
        })
        .collect();
    std::thread::scope(|scope| {
        for lane in 0..8 {
            let readers = &readers;
            scope.spawn(move || {
                for i in (lane..readers.len()).step_by(8).rev() {
                    let mut data = [0u8; 4];
                    readers[i].read_at(0, &mut data).unwrap();
                    assert_eq!(u32::from_le_bytes(data), i as u32);
                    assert_eq!(readers[i].len(), 16);
                }
            });
        }
    });
    // Revisit each source after many evictions; positional reads remain independent.
    for (i, reader) in readers.iter().enumerate() {
        let mut data = [0u8; 4];
        reader.read_at(0, &mut data).unwrap();
        assert_eq!(u32::from_le_bytes(data), i as u32);
    }
    drop(readers);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn evicted_reader_refuses_replaced_content() {
    let _serial = serial();
    let dir = fixture_dir("replacement");
    let path = dir.join("source.bin");
    std::fs::write(&path, b"original").unwrap();
    let original = FileReader::open(&path).unwrap();
    let mut others = Vec::new();
    for i in 0..CAPACITY {
        let path = dir.join(format!("other-{i}.bin"));
        std::fs::write(&path, b"other").unwrap();
        others.push(FileReader::open(&path).unwrap());
    }
    let replacement = dir.join("replacement.bin");
    std::fs::write(&replacement, b"changed!").unwrap();
    std::fs::rename(replacement, &path).unwrap();
    let error = original.read_at(0, &mut [0u8; 8]).unwrap_err();
    assert!(error.to_string().contains("replaced; relink"), "{error}");
    let replacement = FileReader::open(&path).unwrap();
    let mut data = [0u8; 8];
    replacement.read_at(0, &mut data).unwrap();
    assert_eq!(&data, b"changed!");
    drop((original, others, replacement));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dropping_reader_releases_its_cached_file() {
    let _serial = serial();
    let dir = fixture_dir("drop");
    let path = dir.join("source.bin");
    std::fs::write(&path, b"bytes").unwrap();
    let reader = FileReader::open(&path).unwrap();
    let active = reader.file.get().unwrap();
    let weak = Arc::downgrade(&active);
    assert!(weak.upgrade().is_some());
    drop(active);
    drop(reader);
    assert!(weak.upgrade().is_none(), "reader drop must release the cached descriptor immediately");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn media_reader_rejects_a_named_pipe_without_waiting_for_a_writer() {
    let _serial = serial();
    const CHILD: &str = "FILMCRAFT_TEST_PIPE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "reader::linux_files::tests::media_reader_rejects_a_named_pipe_without_waiting_for_a_writer", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "nonregular paths must be rejected");
                return;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("media reader waited for a pipe writer instead of rejecting the path");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let dir = fixture_dir("pipe");
    let path = dir.join("source.bin");
    std::fs::write(&path, b"bytes").unwrap();
    let reader = FileReader::open(&path).unwrap();
    let mut others = Vec::new();
    for i in 0..CAPACITY {
        let other = dir.join(format!("other-{i}.bin"));
        std::fs::write(&other, b"other").unwrap();
        others.push(FileReader::open(&other).unwrap());
    }
    std::fs::rename(&path, dir.join("original.bin")).unwrap();
    assert!(std::process::Command::new("mkfifo").arg(&path).status().unwrap().success());
    assert!(FileReader::open(&path).is_err(), "a pipe is not a media file");
    assert!(reader.read_at(0, &mut [0u8; 5]).is_err(), "an evicted regular file cannot reopen as a pipe");
    drop((reader, others));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn active_read_survives_eviction_and_reader_drop() {
    let _serial = serial();
    use std::os::unix::fs::FileExt;
    let dir = fixture_dir("active");
    let path = dir.join("source.bin");
    std::fs::write(&path, b"bytes").unwrap();
    let reader = FileReader::open(&path).unwrap();
    let active = reader.file.get().unwrap();
    let weak = Arc::downgrade(&active);
    let mut others = Vec::new();
    for i in 0..CAPACITY {
        let path = dir.join(format!("other-{i}.bin"));
        std::fs::write(&path, b"other").unwrap();
        others.push(FileReader::open(&path).unwrap());
    }
    drop(reader);
    let mut bytes = [0u8; 5];
    active.read_exact_at(&mut bytes, 0).unwrap();
    assert_eq!(&bytes, b"bytes");
    drop(active);
    assert!(weak.upgrade().is_none(), "the active descriptor is released when the read finishes");
    drop(others);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cached_moved_file_is_readable_but_evicted_path_needs_relinking() {
    let _serial = serial();
    let dir = fixture_dir("move");
    let path = dir.join("source.bin");
    std::fs::write(&path, b"bytes").unwrap();
    let reader = FileReader::open(&path).unwrap();
    std::fs::rename(&path, dir.join("moved.bin")).unwrap();
    let mut bytes = [0u8; 5];
    reader.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"bytes");
    let mut others = Vec::new();
    for i in 0..CAPACITY {
        let path = dir.join(format!("other-{i}.bin"));
        std::fs::write(&path, b"other").unwrap();
        others.push(FileReader::open(&path).unwrap());
    }
    assert_eq!(reader.read_at(0, &mut bytes).unwrap_err().kind(), io::ErrorKind::NotFound);
    drop((reader, others));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_reads_of_one_evicted_source_keep_offsets_independent() {
    let _serial = serial();
    let dir = fixture_dir("offsets");
    let path = dir.join("source.bin");
    let expected: Vec<u8> = (0..64).collect();
    std::fs::write(&path, &expected).unwrap();
    let reader = FileReader::open(&path).unwrap();
    let mut others = Vec::new();
    for i in 0..CAPACITY {
        let path = dir.join(format!("other-{i}.bin"));
        std::fs::write(&path, b"other").unwrap();
        others.push(FileReader::open(&path).unwrap());
    }
    let start = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for lane in 0..8 {
            let (reader, expected, start) = (&reader, &expected, &start);
            scope.spawn(move || {
                start.wait();
                for offset in [lane * 8, (7 - lane) * 8, lane * 8] {
                    let mut bytes = [0u8; 8];
                    reader.read_at(offset as u64, &mut bytes).unwrap();
                    assert_eq!(&bytes, &expected[offset..offset + 8]);
                }
            });
        }
    });
    drop((reader, others));
    std::fs::remove_dir_all(dir).unwrap();
}
