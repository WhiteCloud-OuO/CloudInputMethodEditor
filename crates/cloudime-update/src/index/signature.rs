use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::UpdateError;

/// 信任的发版公钥（base64 的 32 字节 ed25519 公钥）；换钥时新旧并列一段时间。私钥在 CI 密钥 `CLOUDIME_INDEX_SIGNING_KEY` 里。
pub const PUBLIC_KEYS: &[&str] = &["2qNCMwKGMzaBOsHQLHUornqcopNzmrVBKAmICEl9rkk="];

/// 验索引的分离签名：`signature` 是 `releases.json.sig` 的内容（base64 的 64 字节），任何一把信任的公钥验过就算数。
pub fn verify(message: &[u8], signature: &str) -> Result<(), UpdateError> {
    verify_with(PUBLIC_KEYS, message, signature)
}

pub(crate) fn verify_with(
    keys: &[&str],
    message: &[u8],
    signature: &str,
) -> Result<(), UpdateError> {
    let bytes = STANDARD
        .decode(signature.trim())
        .map_err(|_| UpdateError::MalformedSignature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| UpdateError::MalformedSignature)?;
    let trusted = keys.iter().filter_map(|key| {
        let bytes: [u8; 32] = STANDARD.decode(key).ok()?.try_into().ok()?;
        VerifyingKey::from_bytes(&bytes).ok()
    });
    for key in trusted {
        if key.verify_strict(message, &signature).is_ok() {
            return Ok(());
        }
    }
    Err(UpdateError::UntrustedSignature)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    fn keypair(seed: u8) -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[seed; 32]);
        let public = STANDARD.encode(signing.verifying_key().to_bytes());
        (signing, public)
    }

    #[test]
    fn accepts_a_signature_from_a_trusted_key() {
        let (signing, public) = keypair(7);
        let signature = STANDARD.encode(signing.sign(b"index").to_bytes());
        assert!(verify_with(&[&public], b"index", &format!("{signature}\n")).is_ok());
    }

    #[test]
    fn rejects_tampered_content_and_foreign_keys() {
        let (signing, public) = keypair(7);
        let (_, other) = keypair(9);
        let signature = STANDARD.encode(signing.sign(b"index").to_bytes());
        assert!(matches!(
            verify_with(&[&public], b"index!", &signature),
            Err(UpdateError::UntrustedSignature)
        ));
        assert!(matches!(
            verify_with(&[&other], b"index", &signature),
            Err(UpdateError::UntrustedSignature)
        ));
        assert!(matches!(
            verify_with(&[&public], b"index", "not base64"),
            Err(UpdateError::MalformedSignature)
        ));
    }

    #[test]
    fn embedded_keys_are_well_formed() {
        for key in PUBLIC_KEYS {
            let bytes: [u8; 32] = STANDARD.decode(key).unwrap().try_into().unwrap();
            VerifyingKey::from_bytes(&bytes).unwrap();
        }
    }
}
