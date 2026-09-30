//! Randomness, token hashing and secret encryption. Uses established
//! primitives only: the OS RNG, SHA-256 and XChaCha20-Poly1305.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit};
use iglu_domain::id::PrincipalId;
use iglu_domain::secret::{SecretName, SecretTarget};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("the secret key file must hold 32 bytes as 64 hex digits")]
    BadKey,
    #[error("couldn't read random bytes")]
    Random,
    #[error("a stored secret couldn't be decrypted")]
    Decrypt,
    #[error("couldn't encode a secret's binding")]
    Binding,
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| CryptoError::Random)?;
    Ok(bytes)
}

/// An unguessable token for cookies, codes and CSRF values.
pub fn token() -> Result<String, CryptoError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

pub fn random_u64() -> Result<u64, CryptoError> {
    Ok(u64::from_le_bytes(random_bytes::<8>()?))
}

/// Tokens are stored hashed, so a database leak doesn't leak live sessions.
pub fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The PKCE S256 check: `challenge == base64url(sha256(verifier))`.
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    constant_time_eq(computed.as_bytes(), challenge.as_bytes())
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What a stored secret is bound to, as associated data: its ciphertext opens
/// only for the owner, name and destination it was sealed for, so rows can't
/// be swapped, and a Git credential can't be pointed at another host.
#[derive(Serialize)]
pub struct Binding<'a> {
    pub owner: PrincipalId,
    pub name: &'a SecretName,
    pub target: &'a SecretTarget,
}

impl Binding<'_> {
    fn bytes(&self) -> Result<Vec<u8>, CryptoError> {
        serde_json::to_vec(self).map_err(|_| CryptoError::Binding)
    }
}

/// Encrypts secrets at rest with a key the operator provisions.
pub struct Sealer(XChaCha20Poly1305);

impl Sealer {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = crate::config::read_secret(path)?;
        let key = hex::decode(text).map_err(|_| CryptoError::BadKey)?;
        let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| CryptoError::BadKey)?;
        Ok(Self(cipher))
    }

    /// Returns `(nonce, ciphertext)`.
    pub fn seal(
        &self,
        plaintext: &[u8],
        binding: &Binding<'_>,
    ) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        let aad = binding.bytes()?;
        let nonce = random_bytes::<24>()?;
        let ciphertext = self
            .0
            .encrypt(
                &nonce.into(),
                chacha20poly1305::aead::Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Random)?;
        Ok((nonce.to_vec(), ciphertext))
    }

    pub fn open(
        &self,
        nonce: &[u8],
        ciphertext: &[u8],
        binding: &Binding<'_>,
    ) -> Result<Vec<u8>, CryptoError> {
        let nonce: [u8; 24] = nonce.try_into().map_err(|_| CryptoError::Decrypt)?;
        let aad = binding.bytes()?;
        self.0
            .decrypt(
                &nonce.into(),
                chacha20poly1305::aead::Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Decrypt)
    }
}
