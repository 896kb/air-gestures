//! AirShield transport encryption (kinesis `AirShield.swift`).
//!
//! Record: `0x40 | HMAC-SHA256(counter_le || body)[..8] | body`, where
//! `body = (blocks - 1) as u8 || AES-256-CBC(frame padded with 0xc0+n)`.
//! The IV chains from the previous ciphertext block and the counter wraps.

use aes::Aes256;
use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::error::{perr, Result};

#[derive(Clone)]
pub struct AirShieldKeys {
    pub encryption: [u8; 32],
    pub mac: [u8; 32],
}

impl std::fmt::Debug for AirShieldKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AirShieldKeys { .. }")
    }
}

impl AirShieldKeys {
    /// Parameter-3 key derivation, the only one verified on this band:
    /// `hashed = SHA256(secret)`, `key = HKDF-SHA256(ikm: hashed,
    /// salt: SHA256(hashed || challenge || seed), info: "AirShield", 32)`.
    pub fn derive(secret: &[u8], challenge: &[u8], seed: &[u8]) -> Result<Self> {
        if secret.len() != 32 || challenge.len() != 16 || seed.len() != 32 {
            return Err(perr("Invalid AirShield key material"));
        }
        let hashed = Sha256::digest(secret);
        let salt = Sha256::digest([hashed.as_slice(), challenge, seed].concat());
        let mut key = [0u8; 32];
        Hkdf::<Sha256>::new(Some(&salt), &hashed)
            .expand(b"AirShield", &mut key)
            .expect("32 bytes is a valid HKDF-SHA256 length");
        Ok(Self { encryption: key, mac: key })
    }

    pub fn fixed(encryption: [u8; 32], mac: [u8; 32]) -> Self {
        Self { encryption, mac }
    }
}

pub struct AirShieldCipher {
    keys: AirShieldKeys,
    pub iv: [u8; 16],
    pub counter: u32,
}

impl AirShieldCipher {
    pub fn new(keys: AirShieldKeys, iv: [u8; 16], counter: u32) -> Self {
        Self { keys, iv, counter }
    }

    fn tag(&self, body: &[u8]) -> [u8; 32] {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.keys.mac)
            .expect("HMAC accepts any key length");
        mac.update(&self.counter.to_le_bytes());
        mac.update(body);
        mac.finalize().into_bytes().into()
    }

    pub fn encrypt(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        let count = (16 - frame.len() % 16) % 16;
        let mut padded = frame.to_vec();
        padded.extend(std::iter::repeat_n(0xc0 + count as u8, count));
        if padded.is_empty() || padded.len() > 4096 {
            return Err(perr("AirShield frame too large"));
        }
        let ciphertext = cbc::Encryptor::<Aes256>::new_from_slices(&self.keys.encryption, &self.iv)
            .expect("AES-256 key and 16-byte IV")
            .encrypt_padded_vec_mut::<NoPadding>(&padded);
        let mut body = Vec::with_capacity(ciphertext.len() + 1);
        body.push((ciphertext.len() / 16 - 1) as u8);
        body.extend_from_slice(&ciphertext);
        let tag = self.tag(&body);
        self.iv.copy_from_slice(&ciphertext[ciphertext.len() - 16..]);
        self.counter = self.counter.wrapping_add(1);
        let mut packet = Vec::with_capacity(body.len() + 9);
        packet.push(0x40);
        packet.extend_from_slice(&tag[..8]);
        packet.extend(body);
        Ok(packet)
    }

    /// Verifies the MAC over every byte before decrypting; no unauthenticated
    /// plaintext leaves this type and a failure leaves IV and counter untouched.
    pub fn decrypt(&mut self, packet: &[u8]) -> Result<Vec<u8>> {
        if packet.len() < 10 {
            return Err(perr("Truncated AirShield packet"));
        }
        let expected = self.tag(&packet[9..]);
        let difference = expected[..8]
            .iter()
            .zip(&packet[1..9])
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        if difference != 0 {
            return Err(perr("Band packet authentication failed"));
        }
        let ciphertext = &packet[10..];
        if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
            return Err(perr("Invalid AES block shape"));
        }
        let plaintext = cbc::Decryptor::<Aes256>::new_from_slices(&self.keys.encryption, &self.iv)
            .expect("AES-256 key and 16-byte IV")
            .decrypt_padded_vec_mut::<NoPadding>(ciphertext)
            .map_err(|_| perr("AES operation failed"))?;
        self.iv.copy_from_slice(&ciphertext[ciphertext.len() - 16..]);
        self.counter = self.counter.wrapping_add(1);
        Ok(plaintext)
    }
}

/// Splits the L2CAP byte stream into AirShield records. `0x81/0x82 xx`
/// (xx ≤ 1) and `0x01/0x02 len …` control markers are skipped; `0x41/0x42`
/// relay records are skipped unread; `0x40` records are authenticated and
/// decrypted.
pub struct AirShieldReceiver {
    pub cipher: AirShieldCipher,
    pending: Vec<u8>,
}

impl AirShieldReceiver {
    pub fn new(cipher: AirShieldCipher) -> Self {
        Self { cipher, pending: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.pending.extend_from_slice(bytes);
        let mut plaintext = Vec::new();
        while let Some(&marker) = self.pending.first() {
            let short_control = marker == 0x81 || marker == 0x82;
            if short_control && self.pending.len() >= 2 && self.pending[1] <= 1 {
                self.pending.drain(..2);
                continue;
            }
            if marker == 0x01 || marker == 0x02 {
                if self.pending.len() < 2 {
                    break;
                }
                let size = 3 + usize::from(self.pending[1]);
                if self.pending.len() < size {
                    break;
                }
                self.pending.drain(..size);
                continue;
            }
            if short_control && self.pending.len() < 2 {
                break;
            }
            if !(0x40..=0x42).contains(&marker) {
                return Err(perr("Unsupported band transport marker"));
            }
            if self.pending.len() < 10 {
                break;
            }
            let size = 10 + (usize::from(self.pending[9]) + 1) * 16;
            if self.pending.len() < size {
                break;
            }
            if marker == 0x40 {
                let packet = self.pending[..size].to_vec();
                plaintext.push(self.cipher.decrypt(&packet)?);
            }
            self.pending.drain(..size);
        }
        Ok(plaintext)
    }

    pub fn finish(&self) -> Result<()> {
        if self.pending.is_empty() { Ok(()) } else { Err(perr("Truncated AirShield packet")) }
    }
}
