use crate::{Task, network};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use hpke::{Deserializable, OpModeR, aead::AesGcm256, kdf::HkdfSha256, kem::DhP256HkdfSha256};
use p256::pkcs8::DecodePrivateKey;
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
};

fn fields(blob: &[u8]) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    let mut rest = blob;
    while !rest.is_empty() {
        if rest.len() < 4 {
            bail!("Truncated AEA metadata");
        }
        let length = u32::from_le_bytes(rest[..4].try_into()?) as usize;
        if length < 5 || length > rest.len() {
            bail!("Invalid AEA metadata length");
        }
        let data = &rest[4..length];
        let split = data
            .iter()
            .position(|b| *b == 0)
            .context("Missing AEA metadata separator")?;
        result.insert(
            std::str::from_utf8(&data[..split])?.into(),
            std::str::from_utf8(&data[split + 1..])?
                .trim_end_matches('\0')
                .into(),
        );
        rest = &rest[length..];
    }
    Ok(result)
}
pub fn key<R: Read>(file: &mut R, override_key: Option<&str>, task: &Task) -> Result<Vec<u8>> {
    let mut header = [0u8; 12];
    file.read_exact(&mut header)?;
    if &header[..4] != b"AEA1" || header[4..8] != [1, 0, 0, 0] {
        bail!("Only AEA symmetric profile 1 is currently supported");
    }
    let length = u32::from_le_bytes(header[8..12].try_into()?) as usize;
    if length > 1024 * 1024 {
        bail!("AEA metadata is too large");
    }
    if let Some(value) = override_key {
        let value = STANDARD.decode(value.trim().trim_start_matches("base64:"))?;
        if value.len() != 32 {
            bail!("AEA key must be 32 bytes");
        }
        return Ok(value);
    }
    let mut blob = vec![0; length];
    file.read_exact(&mut blob)?;
    let metadata = fields(&blob)?;
    let url = metadata
        .get("com.apple.wkms.fcs-key-url")
        .context("No public Apple FCS key URL; provide an AEA key")?;
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("wkms-public.apple.com") {
        bail!("FCS URL must use Apple's public WKMS host");
    }
    task.check()?;
    let pem = network::client(true)?
        .get(parsed)
        .send()?
        .error_for_status()?
        .text()?;
    let private = p256::SecretKey::from_pkcs8_pem(&pem).context("Invalid FCS PEM")?;
    let response: serde_json::Value = serde_json::from_str(
        metadata
            .get("com.apple.wkms.fcs-response")
            .context("No FCS response")?,
    )?;
    let enc = STANDARD.decode(crate::required(&response, "enc-request")?)?;
    let wrapped = STANDARD.decode(crate::required(&response, "wrapped-key")?)?;
    let sk = <DhP256HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(&private.to_bytes())
        .map_err(|e| anyhow::anyhow!("HPKE private key: {e:?}"))?;
    let enc = <DhP256HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(&enc)
        .map_err(|e| anyhow::anyhow!("HPKE encapsulation: {e:?}"))?;
    let mut context = hpke::setup_receiver::<AesGcm256, HkdfSha256, DhP256HkdfSha256>(
        &OpModeR::Base,
        &sk,
        &enc,
        b"",
    )
    .map_err(|e| anyhow::anyhow!("HPKE setup: {e:?}"))?;
    let key = context
        .open(&wrapped, b"")
        .map_err(|e| anyhow::anyhow!("FCS key unwrap failed: {e:?}"))?;
    if key.len() != 32 {
        bail!("Invalid unwrapped key length");
    }
    Ok(key)
}
pub fn open(
    mut input: Box<dyn crate::filesystem::ReadSeek>,
    override_key: Option<&str>,
    task: &Task,
) -> Result<Box<dyn crate::filesystem::ReadSeek>> {
    let key = key(&mut input, override_key, task)?;
    input.seek(SeekFrom::Start(0))?;
    let reader = aea_tools::reader::AeaReader::new(&key, input)?;
    Ok(Box::new(aea_tools::stream::AeaStream::new(reader)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_zero_length_field() {
        assert!(fields(&[0, 0, 0, 0]).is_err());
    }
    #[test]
    fn parse_auth_field() {
        assert_eq!(
            fields(&[9, 0, 0, 0, b'k', 0, b'v', b'a', b'l']).unwrap()["k"],
            "val"
        );
    }
}
