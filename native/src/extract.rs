use crate::{Task, aea, required};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
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
    id: String,
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
    stage: String,
}
impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.task.check().map_err(std::io::Error::other)?;
        let n = self.writer.write(data)?;
        self.hash.update(&data[..n]);
        self.bytes += n as u64;
        self.task.progress(&self.stage, self.bytes, self.total);
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
struct Resources<'a> {
    out: &'a Path,
    task: &'a Task,
    indexing: bool,
    flat: bool,
    include: Option<BTreeSet<String>>,
    used: BTreeSet<String>,
    assets: Vec<Asset>,
}
impl Resources<'_> {
    fn destination(&mut self, id: &str) -> Result<PathBuf> {
        let original = safe_relative(id)?;
        if !self.flat || self.indexing {
            return Ok(original);
        }
        let name = original
            .file_name()
            .context("Missing resource filename")?
            .to_string_lossy();
        let file = Path::new(name.as_ref());
        let stem = file.file_stem().unwrap().to_string_lossy();
        let ext = file
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();
        let mut candidate = name.to_string();
        let mut suffix = 2;
        while !self.used.insert(candidate.to_lowercase())
            || self.out.join("assets").join(&candidate).exists()
        {
            candidate = format!("{stem}__{suffix}{ext}");
            suffix += 1;
        }
        Ok(Path::new("assets").join(candidate))
    }
    fn accepts(&self, id: &str) -> bool {
        self.include.as_ref().is_none_or(|set| set.contains(id))
    }
    fn filesystem_file(
        &mut self,
        fs: &mut crate::filesystem::Filesystem,
        source: &str,
        child: &str,
        size: u64,
    ) -> Result<()> {
        let id = Path::new("assets")
            .join(source)
            .join(safe_relative(child)?)
            .to_string_lossy()
            .replace('\\', "/");
        if !self.accepts(&id) {
            return Ok(());
        }
        self.task.check()?;
        let rel = self.destination(&id)?;
        let (bytes, sha256) = if self.indexing {
            self.task.progress(&format!("Indexing {child}"), 0, 0);
            (size, String::new())
        } else {
            let dest = self.out.join(&rel);
            fs::create_dir_all(dest.parent().unwrap())?;
            self.task
                .progress(&format!("Preparing resource {child}"), 0, 0);
            let mut writer = HashingWriter {
                writer: BufWriter::with_capacity(
                    1024 * 1024,
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&dest)?,
                ),
                hash: Sha256::new(),
                bytes: 0,
                task: self.task,
                total: size,
                stage: format!("Extracting {child}"),
            };
            fs.read_file_to(child, &mut writer)?;
            writer.flush()?;
            (writer.bytes, hex::encode(writer.hash.finalize()))
        };
        self.assets.push(Asset {
            id,
            source: child.into(),
            path: rel.to_string_lossy().replace('\\', "/"),
            bytes,
            sha256,
            kind: kind(child).into(),
        });
        Ok(())
    }
}
fn walk(
    fs: &mut crate::filesystem::Filesystem,
    source: &str,
    path: &str,
    resources: &mut Resources<'_>,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        bail!("Directory depth limit exceeded");
    }
    resources.task.check()?;
    resources.task.progress(&format!("Indexing {path}"), 0, 0);
    for entry in fs.list_directory(path)? {
        let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
        safe_relative(&child)?;
        match entry.kind {
            dpp::FsEntryKind::Directory => walk(fs, source, &child, resources, depth + 1)?,
            dpp::FsEntryKind::File => resources.filesystem_file(fs, source, &child, entry.size)?,
            dpp::FsEntryKind::Symlink => {}
        }
    }
    Ok(())
}
pub fn extract(v: &Value, task: &Task) -> Result<Value> {
    let indexing = v["op"] == "index";
    let preview = v["op"] == "preview";
    let include = if let Some(values) = v.get("selected") {
        let values = values.as_array().context("selected must be an array")?;
        let mut set = BTreeSet::new();
        for value in values {
            let id = value.as_str().context("Resource ID must be a string")?;
            safe_relative(id)?;
            set.insert(id.to_owned());
        }
        if set.is_empty() {
            bail!("Select at least one resource");
        }
        Some(set)
    } else {
        None
    };
    if preview && include.as_ref().is_none_or(|set| set.len() != 1) {
        bail!("Preview requires exactly one selected resource");
    }
    let input = Path::new(required(v, "input")?);
    let index_temp = if indexing {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let output_path = match &index_temp {
        Some(temp) => temp.path().join("index"),
        None => PathBuf::from(required(v, "output")?),
    };
    let output = output_path.as_path();
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
    let mut resources = Resources {
        out,
        task,
        indexing,
        flat: v["flat"].as_bool().unwrap_or(false),
        include,
        used: BTreeSet::new(),
        assets: Vec::new(),
    };
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
        let id = Path::new("assets/direct")
            .join(safe_relative(&name)?)
            .to_string_lossy()
            .replace('\\', "/");
        if !resources.accepts(&id) {
            continue;
        }
        let rel = resources.destination(&id)?;
        let size = entry.size();
        let (bytes, sha256) = if indexing {
            (size, String::new())
        } else {
            let dest = out.join(&rel);
            fs::create_dir_all(dest.parent().unwrap())?;
            copy(
                &mut entry,
                &mut OpenOptions::new().write(true).create_new(true).open(dest)?,
                size,
                task,
                "Extracting ZIP resource",
            )?
        };
        resources.assets.push(Asset {
            id,
            source: name.clone(),
            path: rel.to_string_lossy().replace('\\', "/"),
            bytes,
            sha256,
            kind: kind(&name).into(),
        });
    }
    for (i, image) in images.iter().enumerate() {
        task.check()?;
        let source = format!("image-{i}");
        let prefix = format!("assets/{source}/");
        if resources
            .include
            .as_ref()
            .is_some_and(|set| !set.iter().any(|id| id.starts_with(&prefix)))
        {
            continue;
        }
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
        .with_context(|| format!("Failed to open filesystem image: {image}"))?;
        if let Some(set) = resources.include.clone() {
            // Open only selected files; do not walk every resource directory again.
            for id in set.iter().filter(|id| id.starts_with(&prefix)) {
                let child = format!("/{}", id.strip_prefix(&prefix).unwrap());
                if !selected(&child) {
                    bail!("Selection is outside wallpaper roots");
                }
                let relative = safe_relative(&child)?;
                let parent = format!("/{}", relative.parent().unwrap().to_string_lossy());
                let name = relative.file_name().unwrap().to_string_lossy();
                let entry = filesystem
                    .list_directory(&parent)?
                    .into_iter()
                    .find(|entry| entry.name == name && entry.kind == dpp::FsEntryKind::File)
                    .context("Selected resource no longer exists")?;
                resources.filesystem_file(&mut filesystem, &source, &child, entry.size)?;
            }
        } else {
            for root in ROOTS {
                if filesystem.exists(root)? {
                    walk(&mut filesystem, &source, root, &mut resources, 0)?;
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
                            &mut resources,
                            0,
                        )?;
                    }
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
    if let Some(set) = &resources.include {
        let found: BTreeSet<_> = resources
            .assets
            .iter()
            .map(|asset| asset.id.clone())
            .collect();
        if &found != set {
            bail!("One or more selected resources were not found");
        }
    }
    let assets = resources.assets;
    if indexing {
        return Ok(json!({"assets": assets, "images": images}));
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
    let report = json!({"schemaVersion":2,"layout":if resources.flat {"flat"} else {"original"},"input":input.file_name(),"mode":"original-resources","udifFullCrcRequested":v["verifyDisk"].as_bool().unwrap_or(false),"images":images,"assets":assets,"warnings":warnings});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    if preview {
        task.check()?;
        fs::rename(out, output)?;
        return Ok(json!({"output":output,"report":report}));
    }
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
            let mut resources = Resources {
                out: &output,
                task: &task,
                indexing: false,
                flat: false,
                include: None,
                used: BTreeSet::new(),
                assets: Vec::new(),
            };
            walk(&mut filesystem, "image-0", "/", &mut resources, 0).unwrap();
            let assets = resources.assets;
            assert_eq!(
                assets[0].sha256,
                hex::encode(Sha256::digest(b"wallpaper-fixture"))
            );
            assert_eq!(assets[0].bytes, 17);
            let progress = task.state.lock().unwrap();
            assert_eq!(progress["stage"], "Extracting /example.png");
            assert_eq!(progress["done"], 17);
            assert_eq!(progress["total"], 17);
            drop(progress);
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
    fn selection_fixture(input: &Path) {
        let mut zip = ZipWriter::new(File::create(input).unwrap());
        let options = SimpleFileOptions::default();
        zip.start_file("BuildManifest.plist", options).unwrap();
        zip.write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>BuildIdentities</key><array/></dict></plist>").unwrap();
        for (name, data) in [
            ("Library/Wallpaper/A/wall.png", &b"first"[..]),
            ("Library/Wallpaper/B/WALL.png", &b"second"[..]),
            ("Library/Wallpaper/B/assets.car", &b"catalog"[..]),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }
    #[test]
    fn index_select_flatten_and_preview_keep_exact_original_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("fixture.ipsw");
        selection_fixture(&input);
        let unused = temp.path().join("unused");
        let index = extract(
            &json!({"op":"index","input":input,"output":unused}),
            &Task::new(),
        )
        .unwrap();
        assert!(!unused.exists());
        let indexed = index["assets"].as_array().unwrap();
        assert_eq!(indexed.len(), 3);
        assert!(indexed.iter().all(|a| a["sha256"] == ""));
        let ids: Vec<_> = indexed
            .iter()
            .filter(|a| a["kind"] == "image")
            .map(|a| a["id"].clone())
            .collect();
        let output = temp.path().join("flat");
        let result = extract(
            &json!({"op":"extract","input":input,"output":output,"selected":ids,"flat":true}),
            &Task::new(),
        )
        .unwrap();
        assert_eq!(result["report"]["layout"], "flat");
        assert_eq!(result["report"]["assets"].as_array().unwrap().len(), 2);
        assert_eq!(fs::read(output.join("assets/wall.png")).unwrap(), b"first");
        assert_eq!(
            fs::read(output.join("assets/WALL__2.png")).unwrap(),
            b"second"
        );
        let mut zip = ZipArchive::new(File::open(output.join("wallpapers.zip")).unwrap()).unwrap();
        assert_eq!(zip.len(), 3);
        assert!(zip.by_name("assets/WALL__2.png").is_ok());
        assert!(zip.by_name("assets/assets.car").is_err());
        assert_eq!(
            result["report"]["assets"][1]["source"],
            "Library/Wallpaper/B/WALL.png"
        );
        let preview_out = temp.path().join("preview");
        let preview = extract(
            &json!({"op":"preview","input":input,"output":preview_out,"selected":[ids[0]]}),
            &Task::new(),
        )
        .unwrap();
        assert_eq!(preview["report"]["assets"].as_array().unwrap().len(), 1);
        assert!(!preview_out.join("wallpapers.zip").exists());
        let relative = preview["report"]["assets"][0]["path"].as_str().unwrap();
        assert_eq!(fs::read(preview_out.join(relative)).unwrap(), b"first");
    }
    #[test]
    fn invalid_or_empty_selection_leaves_no_output() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("fixture.ipsw");
        selection_fixture(&input);
        for ids in [
            json!([]),
            json!(["assets/direct/Library/Wallpaper/missing.png"]),
            json!(["../../outside"]),
        ] {
            let out = temp.path().join("out");
            assert!(
                extract(
                    &json!({"op":"extract","input":input,"output":out,"selected":ids}),
                    &Task::new()
                )
                .is_err()
            );
            assert!(!out.exists());
        }
    }
    #[test]
    fn filesystem_index_lists_metadata_without_copying_files() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out");
        let mut builder = hfsplus::testutil::HfsPlusImageBuilder::new();
        builder.add_file("example.png", b"wallpaper-fixture", 0o644);
        let task = Task::new();
        let mut fs = crate::filesystem::Filesystem::open(
            Box::new(std::io::Cursor::new(builder.build())),
            &task,
            false,
        )
        .unwrap();
        let mut resources = Resources {
            out: &out,
            task: &task,
            indexing: true,
            flat: false,
            include: None,
            used: BTreeSet::new(),
            assets: Vec::new(),
        };
        walk(&mut fs, "image-0", "/", &mut resources, 0).unwrap();
        assert_eq!(resources.assets[0].bytes, 17);
        assert_eq!(resources.assets[0].sha256, "");
        assert!(!out.exists());
    }
}
