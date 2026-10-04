//! Bounded-memory image opening. Raw APFS/HFS+ avoids UDIF materialization.
use crate::Task;
use anyhow::{Result, bail};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
pub trait ReadSeek: Read + Seek + Unpin {}
impl<T: Read + Seek + Unpin> ReadSeek for T {}
type Reader = Box<dyn ReadSeek>;
/// A stored ZIP entry exposed as a bounded file without duplicating the IPSW.
pub struct FileSlice {
    file: File,
    start: u64,
    length: u64,
    position: u64,
}
impl FileSlice {
    pub fn new(file: File, start: u64, length: u64) -> Result<Self> {
        if start
            .checked_add(length)
            .is_none_or(|n| n > file.metadata().map(|m| m.len()).unwrap_or(0))
        {
            bail!("ZIP entry is outside the input file");
        }
        Ok(Self {
            file,
            start,
            length,
            position: 0,
        })
    }
}
impl Read for FileSlice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min((self.length - self.position) as usize);
        if n == 0 {
            return Ok(0);
        }
        self.file
            .seek(SeekFrom::Start(self.start + self.position))?;
        let read = self.file.read(&mut buf[..n])?;
        self.position += read as u64;
        Ok(read)
    }
}
impl Seek for FileSlice {
    fn seek(&mut self, seek: SeekFrom) -> std::io::Result<u64> {
        let position = match seek {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if position < 0 || position > self.length as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Seek outside ZIP entry",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}
pub enum Filesystem {
    Apfs(dpp::apfs::ApfsVolume<Reader>),
    Hfs(Box<dpp::hfsplus::HfsVolume<Reader>>),
}
impl Filesystem {
    pub fn open(reader: Reader, work: &Path, task: &Task) -> Result<Self> {
        Self::open_inner(reader, work, task, false)
    }
    fn open_inner(mut reader: Reader, work: &Path, task: &Task, raw_only: bool) -> Result<Self> {
        let mut head = [0u8; 36];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut head)?;
        reader.seek(SeekFrom::Start(0))?;
        if &head[32..36] == b"NXSB" {
            return Ok(Self::Apfs(dpp::apfs::ApfsVolume::open(reader)?));
        }
        let mut magic = [0; 2];
        reader.seek(SeekFrom::Start(1024))?;
        reader.read_exact(&mut magic)?;
        reader.seek(SeekFrom::Start(0))?;
        if &magic == b"H+" || &magic == b"HX" {
            return Ok(Self::Hfs(Box::new(dpp::hfsplus::HfsVolume::open(reader)?)));
        }
        if raw_only {
            bail!("Unsupported raw filesystem partition");
        }
        // Default UDIF checksum verification allocates the entire data fork.
        // Validate metadata and verify CRC32 in chunks before opening with that allocation disabled.
        let length = reader.seek(SeekFrom::End(0))?;
        let footer = dpp::udif::KolyHeader::read(&mut reader)?;
        if footer.plist_length > 64 * 1024 * 1024
            || footer
                .plist_offset
                .checked_add(footer.plist_length)
                .is_none_or(|n| n > length)
            || footer
                .data_fork_offset
                .checked_add(footer.data_fork_length)
                .is_none_or(|n| n > length)
        {
            bail!("Invalid UDIF ranges");
        }
        if footer.data_checksum_type == dpp::udif::CHECKSUM_TYPE_CRC32
            && footer.data_checksum[..4] != [0; 4]
        {
            reader.seek(SeekFrom::Start(footer.data_fork_offset))?;
            let mut remain = footer.data_fork_length;
            let mut buffer = vec![0; 1024 * 1024];
            let mut hash = crc32fast::Hasher::new();
            while remain > 0 {
                task.check()?;
                let n = buffer.len().min(remain as usize);
                reader.read_exact(&mut buffer[..n])?;
                hash.update(&buffer[..n]);
                remain -= n as u64;
                task.progress(
                    "Verifying disk image",
                    footer.data_fork_length - remain,
                    footer.data_fork_length,
                );
            }
            if hash.finalize() != u32::from_be_bytes(footer.data_checksum[..4].try_into()?) {
                bail!("UDIF data CRC32 mismatch");
            }
        }
        reader.seek(SeekFrom::Start(0))?;
        let mut dmg = dpp::udif::DmgReader::with_options(
            reader,
            dpp::udif::DmgReaderOptions {
                verify_checksums: false,
            },
        )?;
        let id = dmg.main_partition_id()?;
        // Guard library per-block buffers, including large zero-fill runs.
        let partition = dmg
            .partitions()
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("Missing partition"))?;
        let mut previous = 0;
        for run in &partition.block_map.block_runs {
            let start = run
                .sector_number
                .checked_mul(512)
                .ok_or_else(|| anyhow::anyhow!("Invalid block offset"))?;
            let size = run
                .sector_count
                .checked_mul(512)
                .ok_or_else(|| anyhow::anyhow!("Invalid block size"))?;
            if size > 64 * 1024 * 1024
                || run.compressed_length > 64 * 1024 * 1024
                || start.saturating_sub(previous) > 64 * 1024 * 1024
            {
                bail!("UDIF block exceeds mobile memory budget; this image layout is unsupported");
            }
            previous = start
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("Invalid partition length"))?;
        }
        let raw = work.join("partition.raw");
        let mut writer = CheckedWriter {
            file: File::create(&raw)?,
            task,
            done: 0,
        };
        dmg.decompress_partition_to(id, &mut writer)?;
        writer.file.sync_all()?;
        drop(writer);
        drop(dmg);
        // A decompressed partition is raw APFS or HFS+.
        Self::open_inner(Box::new(File::open(raw)?), work, task, true)
    }
    pub fn exists(&mut self, path: &str) -> Result<bool> {
        Ok(match self {
            Self::Apfs(fs) => fs.exists(path)?,
            Self::Hfs(fs) => fs.exists(path)?,
        })
    }
    pub fn list_directory(&mut self, path: &str) -> Result<Vec<dpp::FsDirEntry>> {
        Ok(match self {
            Self::Apfs(fs) => fs
                .list_directory(path)?
                .iter()
                .map(dpp::FsDirEntry::from)
                .collect(),
            Self::Hfs(fs) => fs
                .list_directory(path)?
                .iter()
                .map(dpp::FsDirEntry::from)
                .collect(),
        })
    }
    pub fn read_file_to<W: Write>(&mut self, path: &str, writer: &mut W) -> Result<u64> {
        Ok(match self {
            Self::Apfs(fs) => fs.read_file_to(path, writer)?,
            Self::Hfs(fs) => fs.read_file_to(path, writer)?,
        })
    }
}
struct CheckedWriter<'a> {
    file: File,
    task: &'a Task,
    done: u64,
}
impl Write for CheckedWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.task.check().map_err(std::io::Error::other)?;
        let n = self.file.write(buffer)?;
        self.done += n as u64;
        self.task.progress("Decoding disk image", self.done, 0);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slice_cannot_escape_zip_entry() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("zip");
        std::fs::write(&path, b"prefixCONTENTsuffix").unwrap();
        let mut slice = FileSlice::new(File::open(path).unwrap(), 6, 7).unwrap();
        let mut data = Vec::new();
        slice.read_to_end(&mut data).unwrap();
        assert_eq!(data, b"CONTENT");
        assert!(slice.seek(SeekFrom::End(1)).is_err());
        assert!(slice.seek(SeekFrom::Start(8)).is_err());
        slice.seek(SeekFrom::Start(0)).unwrap();
        let mut first = [0; 3];
        slice.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"CON");
    }
    #[test]
    fn raw_hfs_and_udif_read_identical_file() {
        let temp = tempfile::tempdir().unwrap();
        let mut builder = hfsplus::testutil::HfsPlusImageBuilder::new();
        builder.add_file("wallpaper.png", b"image-data", 0o644);
        let raw = builder.build();
        let dmg = temp.path().join("test.dmg");
        dpp::udif::DmgBuilder::new()
            .compression(dpp::udif::CompressionMethod::Zlib)
            .add_partition("Apple_HFS", raw.clone())
            .build(&dmg)
            .unwrap();
        for reader in [
            Box::new(std::io::Cursor::new(raw)) as Reader,
            Box::new(File::open(dmg).unwrap()) as Reader,
        ] {
            let mut fs = Filesystem::open(reader, temp.path(), &Task::new()).unwrap();
            let mut bytes = Vec::new();
            fs.read_file_to("/wallpaper.png", &mut bytes).unwrap();
            assert_eq!(bytes, b"image-data");
        }
    }
}
