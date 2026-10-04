use crate::{Task, aea, required};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const ROOTS: &[&str] = &[
    "/Library/Wallpaper",
    "/System/Library/Wallpaper",
    "/System/Library/ProceduralWallpaper",
    "/System/Library/PrivateFrameworks/WallpaperKit.framework",
];
const EXTENSIONS: &[&str] = &[
    "/System/Library/ExtensionKit/Extensions",
    "/System/Library/Extensions",
    "/System/Library/Frameworks/ExtensionKit.framework/Extensions",
    "/System/Library/SpringBoardPlugins",
];
#[derive(Debug, Serialize, Deserialize)]
struct Asset {
    source: String,
    path: String,
    bytes: u64,
    sha256: String,
    kind: String,
}
fn safe_relative(path: &str) -> Result<PathBuf> {
    let trimmed = path.trim_start_matches('/');
    if trimmed.contains('\\') || trimmed.contains(':') || trimmed.is_empty() {
        bail!("Unsafe asset path");
    }
    let p = Path::new(trimmed);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("Unsafe asset path");
    }
    Ok(p.into())
}
fn selected(path: &str) -> bool {
    let p = format!("/{}", path.trim_start_matches('/'));
    ROOTS
        .iter()
        .any(|root| p == *root || p.starts_with(&format!("{root}/")))
        || EXTENSIONS.iter().any(|root| {
            p.strip_prefix(&format!("{root}/")).is_some_and(|s| {
                s.split('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains("poster")
            })
        })
}
fn kind(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "heic" | "heif" => "image",
        "exr" | "usdz" | "metallib" => "procedural",
        "caml" | "ca" => "animation",
        "car" => "asset-catalog",
        _ => "resource",
    }
}
fn manifest(zip: &mut ZipArchive<File>) -> Result<plist::Value> {
    let mut file = zip
        .by_name("BuildManifest.plist")
        .context("IPSW has no BuildManifest.plist")?;
    if file.size() > 32 * 1024 * 1024 {
        bail!("BuildManifest too large");
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(plist::Value::from_reader(std::io::Cursor::new(bytes))?)
}
fn image_paths(m: &plist::Value, device: Option<&str>) -> Result<Vec<String>> {
    let d = m.as_dictionary().context("Invalid BuildManifest")?;
    let ids = d
        .get("BuildIdentities")
        .and_then(|v| v.as_array())
        .context("Missing BuildIdentities")?;
    let mut images = BTreeSet::new();
    for id in ids {
        let Some(id) = id.as_dictionary() else {
            continue;
        };
        if let Some(device) = device {
            let matched = id
                .get("Info")
                .and_then(|v| v.as_dictionary())
                .and_then(|i| i.get("DeviceClass"))
                .and_then(|v| v.as_string());
            if matched != Some(device) {
                continue;
            }
        }
        let Some(parts) = id.get("Manifest").and_then(|v| v.as_dictionary()) else {
            continue;
        };
        for name in ["OS", "SystemOS", "AppOS"] {
            if let Some(path) = parts
                .get(name)
                .and_then(|v| v.as_dictionary())
                .and_then(|v| v.get("Info"))
                .and_then(|v| v.as_dictionary())
                .and_then(|v| v.get("Path"))
                .and_then(|v| v.as_string())
            {
                safe_relative(path)?;
                images.insert(path.to_string());
            }
        }
    }
    Ok(images.into_iter().collect())
}
pub fn inspect(v: &Value) -> Result<Value> {
    let mut zip = ZipArchive::new(File::open(required(v, "input")?)?)?;
    let m = manifest(&mut zip)?;
    let d = m.as_dictionary().context("Invalid manifest")?;
    let images = image_paths(&m, v["board"].as_str())?;
    let mut bytes = 0;
    for path in &images {
        bytes += zip.by_name(path)?.size();
    }
    let boards: BTreeSet<&str> = d
        .get("BuildIdentities")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| {
            v.as_dictionary()?
                .get("Info")?
                .as_dictionary()?
                .get("DeviceClass")?
                .as_string()
        })
        .collect();
    Ok(
        json!({"version":d.get("ProductVersion").and_then(|v|v.as_string()),"build":d.get("ProductBuildVersion").and_then(|v|v.as_string()),"images":images,"boards":boards,"imageBytes":bytes,"note":"Stored ZIP filesystem entries and raw AEA/APFS are read in place. Compressed ZIP or UDIF images may need large temporary files."}),
    )
}
fn copy<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    task: &Task,
    stage: &str,
) -> Result<(u64, String)> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut hash = Sha256::new();
    let mut done = 0;
    loop {
        task.check()?;
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
        done += n as u64;
        task.progress(stage, done, total);
    }
    Ok((done, hex::encode(hash.finalize())))
}
// Compute SHA-256 while the filesystem writes; no second read of each asset.
struct HashingWriter<'a, W> {
    writer: W,
    hash: Sha256,
    bytes: u64,
    task: &'a Task,
    total: u64,
}
impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.task.check().map_err(std::io::Error::other)?;
        let n = self.writer.write(data)?;
        self.hash.update(&data[..n]);
        self.bytes += n as u64;
        self.task
            .progress("Extracting filesystem resource", self.bytes, self.total);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
fn stream_copy<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    task: &Task,
    stage: &str,
) -> Result<u64> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut done = 0;
    loop {
        task.check()?;
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n])?;
        done += n as u64;
        task.progress(stage, done, total);
    }
    Ok(done)
}
fn walk(
    fs: &mut crate::filesystem::Filesystem,
    source: &str,
    path: &str,
    out: &Path,
    assets: &mut Vec<Asset>,
    task: &Task,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        bail!("Directory depth limit exceeded");
    }
    task.check()?;
    for entry in fs.list_directory(path)? {
        let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
        safe_relative(&child)?;
        match entry.kind {
            dpp::FsEntryKind::Directory => walk(fs, source, &child, out, assets, task, depth + 1)?,
            dpp::FsEntryKind::File => {
                // A single resource may be large; the filesystem reader streams it to disk.
                let rel = Path::new("assets")
                    .join(source)
                    .join(safe_relative(&child)?);
                let dest = out.join(&rel);
                fs::create_dir_all(dest.parent().unwrap())?;
                task.progress(&format!("Extracting {child}"), 0, entry.size);
                let mut writer = HashingWriter {
                    writer: BufWriter::with_capacity(1024 * 1024, File::create(&dest)?),
                    hash: Sha256::new(),
                    bytes: 0,
                    task,
                    total: entry.size,
                };
                fs.read_file_to(&child, &mut writer)?;
                writer.flush()?;
                let bytes = writer.bytes;
                let sha256 = hex::encode(writer.hash.finalize());
                assets.push(Asset {
                    source: child.clone(),
                    path: rel.to_string_lossy().replace('\\', "/"),
                    bytes,
                    sha256,
                    kind: kind(&child).into(),
                });
            }
            dpp::FsEntryKind::Symlink => {}
        }
    }
    Ok(())
}
pub fn extract(v: &Value, task: &Task) -> Result<Value> {
    let input = Path::new(required(v, "input")?);
    let output = Path::new(required(v, "output")?);
    if output.exists() {
        bail!("Output folder already exists; choose a new folder");
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix("wallpaper-")
        .tempdir_in(parent)?;
    let out = staging.path();
    let work = tempfile::Builder::new()
        .prefix("ipsw-work-")
        .tempdir_in(parent)?;
    let mut zip = ZipArchive::new(File::open(input)?)?;
    let m = manifest(&mut zip)?;
    let images = image_paths(&m, v["board"].as_str())?;
    let mut assets = Vec::new();
    let mut warnings = Vec::new();
    // Older firmware may store directly accessible wallpaper files in the ZIP.
    let mut seen = BTreeSet::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if !selected(&name) || entry.is_dir() {
            continue;
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            continue;
        }
        if !seen.insert(name.clone()) {
            bail!("Duplicate ZIP resource path");
        }
        let rel = Path::new("assets/direct").join(safe_relative(&name)?);
        let dest = out.join(&rel);
        fs::create_dir_all(dest.parent().unwrap())?;
        let size = entry.size();
        let (bytes, sha256) = copy(
            &mut entry,
            &mut File::create(dest)?,
            size,
            task,
            "Extracting ZIP resource",
        )?;
        assets.push(Asset {
            source: name.clone(),
            path: rel.to_string_lossy().replace('\\', "/"),
            bytes,
            sha256,
            kind: kind(&name).into(),
        });
    }
    for (i, image) in images.iter().enumerate() {
        task.check()?;
        let archive = work.path().join(format!("image-{i}.dmg.aea"));
        let mut entry = zip
            .by_name(image)
            .with_context(|| format!("Missing filesystem image: {image}"))?;
        let mut reader: Box<dyn crate::filesystem::ReadSeek> = if entry.compression()
            == zip::CompressionMethod::Stored
            && entry.size() == entry.compressed_size()
        {
            Box::new(crate::filesystem::FileSlice::new(
                File::open(input)?,
                entry.data_start(),
                entry.size(),
            )?)
        } else {
            let size = entry.size();
            stream_copy(
                &mut entry,
                &mut File::create(&archive)?,
                size,
                task,
                "Unpacking filesystem image",
            )?;
            Box::new(File::open(&archive)?)
        };
        drop(entry);
        if image.ends_with(".aea") {
            task.progress("Opening authenticated Apple archive", 0, 0);
            reader = aea::open(reader, v["aeaKey"].as_str(), task)?;
        }
        task.progress("Opening filesystem", 0, 0);
        let mut filesystem = crate::filesystem::Filesystem::open(
            reader,
            task,
            v["verifyDisk"].as_bool().unwrap_or(false),
        )
        .with_context(|| format!("Unsupported or encrypted filesystem image: {image}"))?;
        let source = format!("image-{i}");
        for root in ROOTS {
            if filesystem.exists(root)? {
                walk(&mut filesystem, &source, root, out, &mut assets, task, 0)?;
            }
        }
        for root in EXTENSIONS {
            if !filesystem.exists(root)? {
                continue;
            }
            for entry in filesystem.list_directory(root)? {
                if entry.kind == dpp::FsEntryKind::Directory
                    && entry.name.to_ascii_lowercase().contains("poster")
                {
                    walk(
                        &mut filesystem,
                        &source,
                        &format!("{root}/{}", entry.name),
                        out,
                        &mut assets,
                        task,
                        0,
                    )?;
                }
            }
        }
        drop(filesystem);
        if archive.exists() {
            fs::remove_file(archive)?;
        }
        let partition = work.path().join("partition.raw");
        if partition.exists() {
            fs::remove_file(partition)?;
        }
    }
    if assets.is_empty() {
        bail!("No wallpaper resources found in the selected filesystem images");
    }
    if assets.iter().any(|a| a.kind == "procedural") {
        warnings.push("Procedural resources were preserved. EXR/USDZ/Metal resources are not flattened wallpapers; native rendering is not implemented.");
    }
    if assets.iter().any(|a| a.kind == "asset-catalog") {
        warnings.push("Assets.car was preserved; catalog decoding is not implemented.");
    }
    warnings.push("Raw extraction does not create PosterBoard databases or install wallpapers. Universal PNG rendering and .tendies conversion are not implemented.");
    let report = json!({"schemaVersion":1,"input":input.file_name(),"mode":"original-resources","udifFullCrcRequested":v["verifyDisk"].as_bool().unwrap_or(false),"images":images,"assets":assets,"warnings":warnings});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    task.progress("Packaging extracted resources", 0, assets.len() as u64);
    let mut archive = ZipWriter::new(BufWriter::with_capacity(
        1024 * 1024,
        File::create(out.join("wallpapers.zip"))?,
    ));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    archive.start_file("report.json", options)?;
    std::io::copy(&mut File::open(out.join("report.json"))?, &mut archive)?;
    for (i, asset) in assets.iter().enumerate() {
        task.check()?;
        archive.start_file(&asset.path, options)?;
        stream_copy(
            &mut File::open(out.join(&asset.path))?,
            &mut archive,
            asset.bytes,
            task,
            "Packaging resource",
        )?;
        task.progress(
            "Packaging extracted resources",
            (i + 1) as u64,
            assets.len() as u64,
        );
    }
    let mut finished = archive.finish()?;
    finished.flush()?;
    finished.get_ref().sync_all()?;
    drop(finished);
    task.check()?;
    fs::rename(out, output)?;
    Ok(json!({"output":output,"archive":output.join("wallpapers.zip"),"report":report}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_contained() {
        for p in ["../../evil", "System/../../evil", "C:\\evil", "/a/../b"] {
            assert!(safe_relative(p).is_err());
        }
        assert_eq!(
            safe_relative("/Library/Wallpaper/a.png").unwrap(),
            PathBuf::from("Library/Wallpaper/a.png")
        );
    }
    #[test]
    fn selected_bundles() {
        assert!(selected(
            "System/Library/ExtensionKit/Extensions/MercuryPosterExtension.appex/vitra.exr"
        ));
        assert!(!selected(
            "System/Library/ExtensionKit/Extensions/Camera.appex/image.png"
        ));
        assert!(!selected("Library/WallpaperEvil/image.png"));
    }
    #[test]
    fn filesystem_writes_hash_correct_bytes_in_both_verification_modes() {
        let temp = tempfile::tempdir().unwrap();
        let mut builder = hfsplus::testutil::HfsPlusImageBuilder::new();
        builder.add_file("example.png", b"wallpaper-fixture", 0o644);
        let disk = temp.path().join("disk.dmg");
        dpp::udif::DmgBuilder::new()
            .compression(dpp::udif::CompressionMethod::Zlib)
            .add_partition("Apple_HFS", builder.build())
            .build(&disk)
            .unwrap();
        for verify in [false, true] {
            let task = Task::new();
            let mut filesystem = crate::filesystem::Filesystem::open(
                Box::new(File::open(&disk).unwrap()),
                &task,
                verify,
            )
            .unwrap();
            let output = temp.path().join(format!("out-{verify}"));
            let mut assets = Vec::new();
            walk(
                &mut filesystem,
                "image-0",
                "/",
                &output,
                &mut assets,
                &task,
                0,
            )
            .unwrap();
            assert_eq!(
                assets[0].sha256,
                hex::encode(Sha256::digest(b"wallpaper-fixture"))
            );
            assert_eq!(assets[0].bytes, 17);
            assert_eq!(
                fs::read(output.join("assets/image-0/example.png")).unwrap(),
                b"wallpaper-fixture"
            );
        }
    }
    #[test]
    fn end_to_end_direct_zip() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("test.ipsw");
        let mut zip = ZipWriter::new(File::create(&input).unwrap());
        let options = SimpleFileOptions::default();
        zip.start_file("BuildManifest.plist", options).unwrap();
        zip.write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>BuildIdentities</key><array/></dict></plist>").unwrap();
        zip.start_file("Library/Wallpaper/example.png", options)
            .unwrap();
        zip.write_all(b"wallpaper-fixture").unwrap();
        zip.finish().unwrap();
        let output = temp.path().join("out");
        let result = extract(&json!({"input":input,"output":output}), &Task::new()).unwrap();
        assert_eq!(
            result["report"]["assets"][0]["sha256"],
            hex::encode(Sha256::digest(b"wallpaper-fixture"))
        );
        assert!(output.join("wallpapers.zip").exists());
        assert_eq!(
            fs::read(output.join("assets/direct/Library/Wallpaper/example.png")).unwrap(),
            b"wallpaper-fixture"
        );
    }
}
