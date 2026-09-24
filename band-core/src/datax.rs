//! DataX framing inside decrypted AirShield records (kinesis `BandWire.frame`, `DataXReceiver`).
//!
//! Frame layout: `u16be size | u16be channel | [u32be words…] | payload`. The
//! size's top bit says typed words follow; each word with its top bit set is
//! followed by another word.

use crate::error::{perr, Result};

pub fn be16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

pub fn be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

pub fn encode_frame(channel: u16, words: &[u32], payload: &[u8]) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(words.len() * 4 + payload.len());
    for word in words {
        body.extend_from_slice(&word.to_be_bytes());
    }
    body.extend_from_slice(payload);
    if body.len() > 0x7fff {
        return Err(perr("DataX frame too large"));
    }
    let size = body.len() as u16 | if words.is_empty() { 0 } else { 0x8000 };
    let mut out = size.to_be_bytes().to_vec();
    out.extend_from_slice(&channel.to_be_bytes());
    out.extend(body);
    Ok(out)
}

/// Debug prints the decrypted payload; never log frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataXFrame {
    pub channel: u16,
    pub words: Vec<u32>,
    pub payload: Vec<u8>,
}

/// Reassembles frames that span several decrypted 16-byte-aligned records.
#[derive(Default)]
pub struct DataXReceiver {
    pending: Vec<u8>,
}

impl std::fmt::Debug for DataXReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataXReceiver").field("pending_len", &self.pending.len()).finish()
    }
}

impl DataXReceiver {
    /// An error leaves the offending bytes pending; treat it as fatal for the
    /// connection and drop the receiver.
    pub fn feed(&mut self, plaintext: &[u8]) -> Result<Vec<DataXFrame>> {
        if plaintext.is_empty() || !plaintext.len().is_multiple_of(16) {
            return Err(perr("Unaligned DataX plaintext"));
        }
        let mut bytes = plaintext;
        let padding = i32::from(*bytes.last().unwrap()) - 0xc0;
        // Only the repeated suffix observed on the tested firmware is stripped.
        if (1..=15).contains(&padding) {
            let count = padding as usize;
            let marker = (0xc0 + padding) as u8;
            if bytes[bytes.len() - count..].iter().all(|&byte| byte == marker) {
                bytes = &bytes[..bytes.len() - count];
            }
        }
        self.pending.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while self.pending.len() >= 4 {
            let length = be16(&self.pending, 0);
            let size = usize::from(length & 0x7fff) + 4;
            if self.pending.len() < size {
                break;
            }
            let channel = be16(&self.pending, 2);
            let mut offset = 4;
            let mut words = Vec::new();
            if length & 0x8000 != 0 {
                loop {
                    if offset + 4 > size {
                        return Err(perr("Truncated DataX typed header"));
                    }
                    let word = be32(&self.pending, offset);
                    words.push(word);
                    offset += 4;
                    if word & 0x8000_0000 == 0 {
                        break;
                    }
                }
            }
            frames.push(DataXFrame { channel, words, payload: self.pending[offset..size].to_vec() });
            self.pending.drain(..size);
        }
        Ok(frames)
    }
}
