//! Bounded cache for repeated filesystem metadata reads. Large payload reads bypass it.
use lru::LruCache;
use std::{
    io::{Read, Seek, SeekFrom},
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
const BUDGET: usize = 32 * 1024 * 1024;
pub struct MetadataCache<R> {
    reader: R,
    position: u64,
    cache: LruCache<(u64, usize), Vec<u8>>,
    bytes: usize,
    cancelled: Arc<AtomicBool>,
}
impl<R: Read + Seek> MetadataCache<R> {
    pub fn new(mut reader: R, cancelled: Arc<AtomicBool>) -> std::io::Result<Self> {
        let position = reader.stream_position()?;
        Ok(Self {
            reader,
            position,
            cache: LruCache::new(NonZeroUsize::new(8192).unwrap()),
            bytes: 0,
            cancelled,
        })
    }
}
impl<R: Read + Seek> Read for MetadataCache<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("Cancelled"));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let key = (self.position, buffer.len());
        let small = buffer.len() <= 16384;
        if small && let Some(data) = self.cache.get(&key) {
            buffer[..data.len()].copy_from_slice(data);
            self.position += data.len() as u64;
            return Ok(data.len());
        }
        self.reader.seek(SeekFrom::Start(self.position))?;
        let n = self.reader.read(buffer)?;
        self.position += n as u64;
        if small && n > 0 {
            while self.bytes + n > BUDGET || self.cache.len() == self.cache.cap().get() {
                let Some((_, data)) = self.cache.pop_lru() else {
                    break;
                };
                self.bytes -= data.len();
            }
            self.cache.put(key, buffer[..n].to_vec());
            self.bytes += n;
        }
        Ok(n)
    }
}
impl<R: Read + Seek> Seek for MetadataCache<R> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(p) => u64::try_from(self.position as i128 + p as i128)
                .map_err(|_| std::io::Error::other("Invalid seek"))?,
            SeekFrom::End(p) => self.reader.seek(SeekFrom::End(p))?,
        };
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Counted {
        data: std::io::Cursor<Vec<u8>>,
        reads: usize,
    }
    impl Read for Counted {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            self.data.read(b)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, s: SeekFrom) -> std::io::Result<u64> {
            self.data.seek(s)
        }
    }
    #[test]
    fn repeated_metadata_reads_hit_cache_and_payloads_do_not_evict_it() {
        let reader = Counted {
            data: std::io::Cursor::new(vec![42; 1024 * 1024]),
            reads: 0,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut cache = MetadataCache::new(reader, cancelled.clone()).unwrap();
        let mut meta = [0; 4096];
        cache.read_exact(&mut meta).unwrap();
        cache.seek(SeekFrom::Start(65536)).unwrap();
        cache.read_exact(&mut vec![0; 65536]).unwrap();
        cache.seek(SeekFrom::Start(0)).unwrap();
        cache.read_exact(&mut meta).unwrap();
        assert_eq!(cache.reader.reads, 2);
        assert_eq!(meta, [42; 4096]);
        cancelled.store(true, Ordering::Relaxed);
        cache.seek(SeekFrom::Start(0)).unwrap();
        assert!(cache.read(&mut meta).is_err());
    }
}
