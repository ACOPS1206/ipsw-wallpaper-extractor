//! Seekable UDIF partition: decode only blocks actually requested by the filesystem.
use crate::{Task, filesystem::ReadSeek};
use anyhow::{Result, bail};
use dpp::udif::{BlockType, DmgReader, DmgReaderOptions, KolyHeader, format::BlockRun};
use lru::LruCache;
use std::{
    io::{Read, Seek, SeekFrom},
    num::NonZeroUsize,
};

const LIMIT: u64 = 64 * 1024 * 1024;
struct Run {
    start: u64,
    end: u64,
    data: u64,
    block: BlockRun,
}
pub struct LazyUdif {
    reader: Box<dyn ReadSeek>,
    runs: Vec<Run>,
    length: u64,
    position: u64,
    cache: LruCache<usize, Vec<u8>>,
    cached_bytes: usize,
}
impl LazyUdif {
    pub fn open(mut reader: Box<dyn ReadSeek>, task: &Task, verify: bool) -> Result<Self> {
        let length = reader.seek(SeekFrom::End(0))?;
        let footer = KolyHeader::read(&mut reader)?;
        if footer.plist_length > LIMIT
            || footer
                .plist_offset
                .checked_add(footer.plist_length)
                .is_none_or(|end| end > length)
            || footer
                .data_fork_offset
                .checked_add(footer.data_fork_length)
                .is_none_or(|end| end > length)
        {
            bail!("Invalid UDIF ranges");
        }
        if verify
            && footer.data_checksum_type == dpp::udif::CHECKSUM_TYPE_CRC32
            && footer.data_checksum[..4] != [0; 4]
        {
            reader.seek(SeekFrom::Start(footer.data_fork_offset))?;
            let mut buffer = vec![0; 1024 * 1024];
            let mut hash = crc32fast::Hasher::new();
            let mut done = 0;
            while done < footer.data_fork_length {
                task.check()?;
                let n = buffer.len().min((footer.data_fork_length - done) as usize);
                reader.read_exact(&mut buffer[..n])?;
                hash.update(&buffer[..n]);
                done += n as u64;
                task.progress("Verifying disk image", done, footer.data_fork_length);
            }
            if hash.finalize() != u32::from_be_bytes(footer.data_checksum[..4].try_into()?) {
                bail!("UDIF data CRC32 mismatch");
            }
        }
        let partition = {
            let dmg = DmgReader::with_options(
                &mut reader,
                DmgReaderOptions {
                    verify_checksums: false,
                },
            )?;
            let id = dmg.main_partition_id()?;
            dmg.partitions()
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| anyhow::anyhow!("Missing partition"))?
                .clone()
        };
        let length = partition
            .block_map
            .sector_count
            .checked_mul(512)
            .ok_or_else(|| anyhow::anyhow!("Invalid partition size"))?;
        let mut runs = Vec::new();
        let fork_end = footer.data_fork_offset + footer.data_fork_length;
        for block in partition.block_map.block_runs {
            if matches!(block.block_type, BlockType::Comment | BlockType::End)
                || block.sector_count == 0
            {
                continue;
            }
            let start = block
                .sector_number
                .checked_mul(512)
                .ok_or_else(|| anyhow::anyhow!("Invalid block offset"))?;
            let size = block
                .sector_count
                .checked_mul(512)
                .ok_or_else(|| anyhow::anyhow!("Invalid block size"))?;
            let end = start
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("Invalid block end"))?;
            let data = footer
                .data_fork_offset
                .checked_add(block.compressed_offset)
                .ok_or_else(|| anyhow::anyhow!("Invalid compressed offset"))?;
            if end > length
                || (block.compressed_length > 0
                    && data
                        .checked_add(block.compressed_length)
                        .is_none_or(|n| n > fork_end))
            {
                bail!("UDIF block outside partition or data fork");
            }
            match block.block_type {
                BlockType::Raw | BlockType::Ignore if block.compressed_length > size => {
                    bail!("UDIF raw block exceeds declared size")
                }
                BlockType::ZeroFill | BlockType::Raw | BlockType::Ignore => {}
                _ if size > LIMIT || block.compressed_length > LIMIT => {
                    bail!("UDIF compressed block exceeds mobile memory budget")
                }
                _ => {}
            }
            runs.push(Run {
                start,
                end,
                data,
                block,
            });
        }
        runs.sort_by_key(|r| r.start);
        if runs.windows(2).any(|pair| pair[0].end > pair[1].start) {
            bail!("Overlapping UDIF blocks");
        }
        Ok(Self {
            reader,
            runs,
            length,
            position: 0,
            cache: LruCache::new(NonZeroUsize::new(256).unwrap()),
            cached_bytes: 0,
        })
    }
    fn decode(&mut self, index: usize) -> Result<()> {
        if self.cache.contains(&index) {
            return Ok(());
        }
        let run = &self.runs[index];
        self.reader.seek(SeekFrom::Start(run.data))?;
        let mut compressed = vec![0; run.block.compressed_length as usize];
        self.reader.read_exact(&mut compressed)?;
        let size = (run.end - run.start) as usize;
        // Evict before allocating another decoded block.
        while self.cached_bytes + size > LIMIT as usize
            || self.cache.len() == self.cache.cap().get()
        {
            let Some((_, old)) = self.cache.pop_lru() else {
                break;
            };
            self.cached_bytes -= old.len();
        }
        let bytes = match run.block.block_type {
            BlockType::Zlib => decode_exact(flate2::read::ZlibDecoder::new(&compressed[..]), size)?,
            BlockType::Bzip2 => decode_exact(bzip2::read::BzDecoder::new(&compressed[..]), size)?,
            BlockType::Xz => decode_exact(lzma_rust2::XzReader::new(&compressed[..], false), size)?,
            BlockType::Lzfse => {
                let mut output = BoundedOutput {
                    data: Vec::with_capacity(size),
                    size,
                };
                lzfse_rust::LzfseRingDecoder::default()
                    .decode(&mut &compressed[..], &mut output)?;
                if output.data.len() != size {
                    bail!("LZFSE block length mismatch");
                }
                output.data
            }
            _ => bail!("Unsupported UDIF compression"),
        };
        self.cached_bytes += bytes.len();
        self.cache.put(index, bytes);
        Ok(())
    }
}
fn decode_exact<R: Read>(mut decoder: R, size: usize) -> Result<Vec<u8>> {
    let mut data = vec![0; size];
    decoder.read_exact(&mut data)?;
    if decoder.read(&mut [0; 1])? != 0 {
        bail!("UDIF block decoded beyond declared size");
    }
    Ok(data)
}
struct BoundedOutput {
    data: Vec<u8>,
    size: usize,
}
impl std::io::Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.size - self.data.len() {
            return Err(std::io::Error::other(
                "UDIF block decoded beyond declared size",
            ));
        }
        self.data.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Read for LazyUdif {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        let mut written = 0;
        while written < buffer.len() && self.position < self.length {
            let i = self.runs.partition_point(|r| r.end <= self.position);
            if i == self.runs.len() || self.runs[i].start > self.position {
                let end = self.runs.get(i).map_or(self.length, |r| r.start);
                let n = (buffer.len() - written).min((end - self.position) as usize);
                buffer[written..written + n].fill(0);
                written += n;
                self.position += n as u64;
                continue;
            }
            let run = &self.runs[i];
            let local = self.position - run.start;
            let mut n = (buffer.len() - written).min((run.end - self.position) as usize);
            match run.block.block_type {
                BlockType::ZeroFill => buffer[written..written + n].fill(0),
                BlockType::Raw | BlockType::Ignore => {
                    if local >= run.block.compressed_length {
                        buffer[written..written + n].fill(0);
                    } else {
                        n = n.min((run.block.compressed_length - local) as usize);
                        self.reader.seek(SeekFrom::Start(run.data + local))?;
                        self.reader.read_exact(&mut buffer[written..written + n])?;
                    }
                }
                _ => {
                    self.decode(i).map_err(std::io::Error::other)?;
                    let data = self.cache.get(&i).unwrap();
                    buffer[written..written + n]
                        .copy_from_slice(&data[local as usize..local as usize + n]);
                }
            }
            written += n;
            self.position += n as u64;
        }
        Ok(written)
    }
}
impl Seek for LazyUdif {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let p = match from {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::Current(p) => self.position as i128 + p as i128,
            SeekFrom::End(p) => self.length as i128 + p as i128,
        };
        if p < 0 || p > self.length as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Seek outside UDIF partition",
            ));
        }
        self.position = p as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };
    struct Counted {
        file: File,
        bytes: Arc<AtomicU64>,
    }
    impl Read for Counted {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = self.file.read(b)?;
            self.bytes.fetch_add(n as u64, Ordering::Relaxed);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, s: SeekFrom) -> std::io::Result<u64> {
            self.file.seek(s)
        }
    }
    #[test]
    fn random_reads_equal_full_decode_and_do_not_scan_entire_disk() {
        let temp = tempfile::tempdir().unwrap();
        let source: Vec<u8> = (0..8 * 1024 * 1024)
            .map(|i| ((i * 31 + i / 4096) % 251) as u8)
            .collect();
        for method in [
            dpp::udif::CompressionMethod::Raw,
            dpp::udif::CompressionMethod::Zlib,
            dpp::udif::CompressionMethod::Bzip2,
            dpp::udif::CompressionMethod::Lzfse,
        ] {
            let path = temp.path().join("fixture.dmg");
            dpp::udif::DmgBuilder::new()
                .compression(method)
                .chunk_size(65536)
                .add_partition("Apple_APFS", source.clone())
                .build(&path)
                .unwrap();
            let mut reference = dpp::udif::DmgReader::new(File::open(&path).unwrap()).unwrap();
            assert_eq!(reference.decompress_main_partition().unwrap(), source);
            let reads = Arc::new(AtomicU64::new(0));
            let mut disk = LazyUdif::open(
                Box::new(Counted {
                    file: File::open(&path).unwrap(),
                    bytes: reads.clone(),
                }),
                &Task::new(),
                false,
            )
            .unwrap();
            for offset in [0, 65500, 7 * 1024 * 1024 + 13] {
                disk.seek(SeekFrom::Start(offset)).unwrap();
                let mut bytes = [0; 512];
                disk.read_exact(&mut bytes).unwrap();
                assert_eq!(bytes, source[offset as usize..offset as usize + 512]);
            }
            assert!(
                reads.load(Ordering::Relaxed) < std::fs::metadata(&path).unwrap().len() / 2,
                "on-demand reads scanned too much"
            );
            let before = reads.load(Ordering::Relaxed);
            disk.seek(SeekFrom::Start(7 * 1024 * 1024 + 13)).unwrap();
            disk.read_exact(&mut [0; 512]).unwrap();
            if method != dpp::udif::CompressionMethod::Raw {
                assert_eq!(reads.load(Ordering::Relaxed), before);
            }
            disk.seek(SeekFrom::End(-3)).unwrap();
            let mut end = [0; 16];
            assert_eq!(disk.read(&mut end).unwrap(), 3);
            assert!(disk.seek(SeekFrom::End(1)).is_err());
        }
    }
    #[test]
    fn optional_full_crc_rejects_damage_in_unread_data() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("corrupt.dmg");
        dpp::udif::DmgBuilder::new()
            .compression(dpp::udif::CompressionMethod::Raw)
            .add_partition("Apple_APFS", vec![1; 2 * 1024 * 1024])
            .build(&path)
            .unwrap();
        let footer = KolyHeader::read(&mut File::open(&path).unwrap()).unwrap();
        let mut data = std::fs::read(&path).unwrap();
        data[footer.data_fork_offset as usize + 1024 * 1024] ^= 1;
        std::fs::write(&path, data).unwrap();
        assert!(LazyUdif::open(Box::new(File::open(&path).unwrap()), &Task::new(), true).is_err());
        let mut fast =
            LazyUdif::open(Box::new(File::open(&path).unwrap()), &Task::new(), false).unwrap();
        let mut data = [0; 512];
        fast.read_exact(&mut data).unwrap();
        assert_eq!(data, [1; 512]);
    }
    #[test]
    fn large_zero_runs_are_virtual() {
        let block = BlockRun {
            block_type: BlockType::ZeroFill,
            comment: 0,
            sector_number: 0,
            sector_count: 1024 * 1024,
            compressed_offset: 0,
            compressed_length: 0,
        };
        let mut disk = LazyUdif {
            reader: Box::new(std::io::Cursor::new(Vec::<u8>::new())),
            runs: vec![Run {
                start: 0,
                end: 512 * 1024 * 1024,
                data: 0,
                block,
            }],
            length: 512 * 1024 * 1024,
            position: 0,
            cache: LruCache::new(NonZeroUsize::new(2).unwrap()),
            cached_bytes: 0,
        };
        disk.seek(SeekFrom::End(-512)).unwrap();
        let mut bytes = [1; 512];
        disk.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [0; 512]);
        assert_eq!(disk.cached_bytes, 0);
    }
}
