use anyhow::{Result, bail, Context};
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde_json::{Value,json};
use sha1::{Digest, Sha1};
use std::{fs::{self, File, OpenOptions}, io::{Read,Write}, path::Path, time::Duration};
use crate::{Task,required};

pub fn apple_url(url: &Url) -> bool {
    let host=url.host_str().unwrap_or("");
    url.scheme()=="https" && url.username().is_empty() && url.password().is_none() && url.port_or_known_default()==Some(443) &&
        (host=="apple.com" || host.ends_with(".apple.com") || host=="cdn-apple.com" || host.ends_with(".cdn-apple.com"))
}
pub fn client(apple_only: bool) -> Result<Client> {
    Ok(Client::builder().connect_timeout(Duration::from_secs(20)).timeout(Duration::from_secs(60))
        .redirect(Policy::custom(move |attempt| {
            if attempt.previous().len()>=5 { attempt.error("Too many redirects") }
            else if apple_only && !apple_url(attempt.url()) { attempt.error("Redirect left Apple CDN") }
            else { attempt.follow() }
        })).build()?)
}
pub fn devices(task:&Task)->Result<Value> {
    task.progress("Loading device catalog",0,0);
    let devices:Value=client(false)?.get("https://api.ipsw.me/v4/devices").send()?.error_for_status()?.json()?;
    let mut devices:Vec<Value>=devices.as_array().context("Invalid device catalog")?.iter().filter(|d| d["identifier"].as_str().is_some_and(|s|s.starts_with("iPhone")||s.starts_with("iPad"))).cloned().collect();
    devices.sort_by(|a,b|a["name"].as_str().cmp(&b["name"].as_str())); Ok(json!(devices))
}
pub fn firmwares(device:&str,task:&Task)->Result<Value> {
    if !device.bytes().all(|c|c.is_ascii_alphanumeric()||c==b',') { bail!("Invalid device identifier"); }
    task.progress("Loading firmware catalog",0,0);
    let data:Value=client(false)?.get(format!("https://api.ipsw.me/v4/device/{device}?type=ipsw")).send()?.error_for_status()?.json()?;
    let firmware:Vec<Value>=data["firmwares"].as_array().context("Invalid firmware catalog")?.iter().filter(|f|f["url"].as_str().and_then(|s|Url::parse(s).ok()).is_some_and(|u|apple_url(&u))).cloned().collect();
    Ok(json!(firmware))
}
pub fn download(v:&Value,task:&Task)->Result<Value> {
    let url=Url::parse(required(v,"url")?)?;
    if !apple_url(&url) { bail!("Use an HTTPS Apple CDN URL"); }
    let dest=Path::new(required(v,"output")?);
    if dest.exists() { bail!("Destination already exists"); }
    if let Some(parent)=dest.parent() { if !parent.as_os_str().is_empty(){fs::create_dir_all(parent)?;} }
    let part=dest.with_extension("ipsw.part"); let sidecar=dest.with_extension("ipsw.part.json");
    let identity=json!({"url":url.as_str(),"sha1":v["sha1"],"size":v["size"]});
    let old=fs::read(&sidecar).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok());
    if part.exists() && old.as_ref()!=Some(&identity) { bail!("Partial file belongs to another download; remove it before retrying"); }
    fs::write(&sidecar,serde_json::to_vec(&identity)?)?;
    let offset=part.metadata().map(|m|m.len()).unwrap_or(0);
    let mut request=client(true)?.get(url);
    if offset>0 { request=request.header("Range",format!("bytes={offset}-")); }
    let mut response=request.send()?;
    let mut done=0; let mut append=false;
    if response.status()==reqwest::StatusCode::PARTIAL_CONTENT {
        let range=response.headers().get("content-range").context("Missing Content-Range")?.to_str()?;
        if !range.starts_with(&format!("bytes {offset}-")) { bail!("Server resumed at wrong offset"); }
        done=offset; append=true;
    } else { response.error_for_status_ref()?; }
    let total=response.content_length().map(|n|n+done).unwrap_or(v["size"].as_u64().unwrap_or(0));
    let mut output=OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&part)?;
    let mut buf=vec![0u8;1024*1024];
    loop {task.check()?; let n=response.read(&mut buf)?; if n==0 {break;} output.write_all(&buf[..n])?; done+=n as u64; task.progress("Downloading from Apple CDN",done,total);}
    output.sync_all()?; drop(output);
    if total>0 && done!=total {bail!("Download truncated: {done}/{total}");}
    if let Some(expected)=v["size"].as_u64().filter(|n|*n>0) {if done!=expected{bail!("Catalog size mismatch");}}
    let mut verified=false;
    if let Some(expected)=v["sha1"].as_str().filter(|s|!s.is_empty()) {
        let mut hash=Sha1::new();let mut file=File::open(&part)?;let mut checked=0;
        loop {task.check()?;let n=file.read(&mut buf)?;if n==0{break;}hash.update(&buf[..n]);checked+=n as u64;task.progress("Verifying SHA-1",checked,done);}
        if hex::encode(hash.finalize())!=expected.to_ascii_lowercase(){bail!("SHA-1 mismatch; partial file retained for diagnosis");} verified=true;
    }
    fs::rename(&part,dest)?;fs::remove_file(sidecar)?;
    Ok(json!({"path":dest,"bytes":done,"checksumVerified":verified}))
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn cdn_host_validation(){for u in ["https://updates.cdn-apple.com/a.ipsw","https://secure-appldnld.apple.com/a.ipsw"]{assert!(apple_url(&Url::parse(u).unwrap()));}for u in ["https://apple.com.evil.test/x","http://updates.cdn-apple.com/x","https://evilapple.com/x"]{assert!(!apple_url(&Url::parse(u).unwrap()));}}
}
