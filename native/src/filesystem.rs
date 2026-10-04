//! Bounded-memory image opening. Raw APFS/HFS+ avoids UDIF materialization.
use crate::Task;
use anyhow::{Result, bail};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
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
    pub fn open(reader: Reader, task: &Task, verify: bool) -> Result<Self> {
        Self::open_inner(reader, task, verify, false)
    }
    fn open_inner(mut reader: Reader, task: &Task, verify: bool, raw_only: bool) -> Result<Self> {
        task.check()?;
        let mut head = [0u8; 36];
        reader.seek(SeekFrom::Start(0))?;
        reader.read_exact(&mut head)?;
        reader.seek(SeekFrom::Start(0))?;
        if &head[32..36] == b"NXSB" {
            let cached: Reader = Box::new(crate::read_cache::MetadataCache::new(
                reader,
                task.cancelled.clone(),
            )?);
            return Ok(Self::Apfs(dpp::apfs::ApfsVolume::open(cached)?));
        }
        let mut magic = [0; 2];
        reader.seek(SeekFrom::Start(1024))?;
        reader.read_exact(&mut magic)?;
        reader.seek(SeekFrom::Start(0))?;
        if &magic == b"H+" || &magic == b"HX" {
            let cached: Reader = Box::new(crate::read_cache::MetadataCache::new(
                reader,
                task.cancelled.clone(),
            )?);
            return Ok(Self::Hfs(Box::new(dpp::hfsplus::HfsVolume::open(cached)?)));
        }
        if raw_only {
            bail!("Unsupported raw filesystem partition");
        }
        task.progress("Opening disk image on demand", 0, 0);
        let disk = crate::udif::LazyUdif::open(reader, task, verify)?;
        Self::open_inner(Box::new(disk), task, verify, true)
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
            let mut fs = Filesystem::open(reader, &Task::new(), false).unwrap();
            let mut bytes = Vec::new();
            fs.read_file_to("/wallpaper.png", &mut bytes).unwrap();
            assert_eq!(bytes, b"image-data");
        }
    }
}
