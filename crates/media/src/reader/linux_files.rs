//! Linux file descriptors are shared across media readers and reopened on demand.
//! An inode check refuses replacement content when a cached descriptor was evicted.

use std::collections::{HashMap, VecDeque};
use std::fs::{File, Metadata};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const CAPACITY: usize = 32;

pub(super) fn open_file(path: &Path) -> io::Result<File> {
    // A path replaced by a pipe must fail the regular-file check without waiting for a writer.
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)
}

#[derive(Default)]
struct Files {
    next_id: u64,
    open: HashMap<u64, Arc<File>>,
    order: VecDeque<u64>,
}

impl Files {
    fn touch(&mut self, id: u64) {
        self.order.retain(|old| *old != id);
        self.order.push_back(id);
    }

    fn insert(&mut self, id: u64, file: Arc<File>) {
        self.open.insert(id, file);
        self.touch(id);
        while self.open.len() > CAPACITY {
            let Some(old) = self.order.pop_front() else { break };
            self.open.remove(&old);
        }
    }
}

fn files() -> &'static Mutex<Files> {
    static FILES: OnceLock<Mutex<Files>> = OnceLock::new();
    FILES.get_or_init(Mutex::default)
}

#[derive(Debug)]
pub(super) struct FileHandle {
    id: u64,
    path: PathBuf,
    device: u64,
    inode: u64,
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    reopen: Mutex<()>,
}

impl FileHandle {
    pub(super) fn new(path: &Path, file: File, metadata: &Metadata) -> io::Result<Self> {
        let path = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
        let mut cache = files().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = cache.next_id;
        cache.next_id = id.checked_add(1).ok_or_else(|| io::Error::other("media reader identifiers exhausted"))?;
        cache.insert(id, Arc::new(file));
        Ok(Self {
            id,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            len: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            reopen: Mutex::new(()),
        })
    }

    pub(super) fn get(&self) -> io::Result<Arc<File>> {
        let mut cache = files().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(file) = cache.open.get(&self.id).cloned() {
            cache.touch(self.id);
            return Ok(file);
        }
        // Coalesce simultaneous misses without holding the global cache lock over storage I/O.
        drop(cache);
        let _reopen = self.reopen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut cache = files().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(file) = cache.open.get(&self.id).cloned() {
            cache.touch(self.id);
            return Ok(file);
        }
        drop(cache);
        let file = open_file(&self.path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
            || metadata.len() != self.len
            || (metadata.mtime(), metadata.mtime_nsec()) != self.modified
            || (metadata.ctime(), metadata.ctime_nsec()) != self.changed
        {
            return Err(io::Error::other(format!("{} changed or was replaced; relink the media before reading it", self.path.display())));
        }
        let file = Arc::new(file);
        let mut cache = files().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.insert(self.id, file.clone());
        Ok(file)
    }
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        let mut cache = files().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.open.remove(&self.id);
        cache.order.retain(|old| *old != self.id);
    }
}

#[cfg(test)]
#[path = "linux_files_tests.rs"]
mod tests;
