//! The TUR USB packet: a 500-byte command, DES-CBC encrypted, framed in 512
//! bytes with a two-byte trailer.
//!
//! The bytes are the whole protocol: three independent drivers agree on them,
//! so they are built and tested here without touching USB.

use cbc::Encryptor;
use cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use des::Des;

type EncryptorDes = Encryptor<Des>;

pub const COMMAND_SIZE: usize = 500;
pub const PACKET_SIZE: usize = 512;
/// The key is also the initialisation vector, read from L-Connect's own
/// traffic rather than guessed.
pub const PACKET_KEY: [u8; 8] = *b"slv3tuzx";
/// Bytes 2–3 of every command, constant across commands and drivers.
pub const MAGIC: [u8; 2] = [0x1a, 0x6d];
/// The frame's last two bytes. A packet without them is refused by the panel.
pub const TRAILER: [u8; 2] = [0xa1, 0x1a];

/// Only non-destructive opcodes are named: firmware and storage writes are
/// deliberately absent rather than merely unused.
pub mod opcode {
    pub const VERSION: u8 = 10;
    pub const BRIGHTNESS: u8 = 14;
    pub const FRAME_RATE: u8 = 15;
    /// The panel's own quarter-turn state; it persists across power cycles, so
    /// the driver pins it rather than inheriting the vendor app's.
    pub const ROTATE: u8 = 13;
    pub const PUSH_JPEG: u8 = 101;
    pub const PUSH_PNG: u8 = 102;
    /// L-Connect's `FB1_State`: parameter 1 shows the PNG layer, 0 hides it.
    pub const OVERLAY_LAYER: u8 = 105;
    /// Ends any playback still running before a pushed frame.
    pub const STOP_PLAY: u8 = 123;
    pub const BUFFER_LEVEL: u8 = 122;
    /// One block of an H.264 elementary stream the panel decodes itself.
    pub const PLAY_BLOCK: u8 = 121;
    /// The block size this panel wants its H.264 stream cut into.
    pub const H264_BLOCK: u8 = 17;
}

/// The fields a command may carry.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fields {
    /// The payload length, big-endian at offset 8, for a command with an image.
    pub payload_length: Option<u32>,
    /// One byte at offset 8, for brightness and frame rate.
    pub parameter: Option<u8>,
    pub last_block: Option<bool>,
    pub play_count: Option<u8>,
}

/// The plaintext command block, before encryption.
///
/// `elapsed_ms` becomes a little-endian 32-bit timestamp at offset 4. The device
/// does not verify it, so a since-launch value is sound — which is what every
/// driver sends.
pub fn command_block(opcode: u8, elapsed_ms: u32, fields: &Fields) -> [u8; COMMAND_SIZE] {
    let mut block = [0_u8; COMMAND_SIZE];
    block[0] = opcode;
    block[2..4].copy_from_slice(&MAGIC);
    block[4..8].copy_from_slice(&elapsed_ms.to_le_bytes());
    if let Some(length) = fields.payload_length {
        block[8..12].copy_from_slice(&length.to_be_bytes());
    }
    if let Some(parameter) = fields.parameter {
        block[8] = parameter;
    }
    if let Some(last) = fields.last_block {
        block[12] = u8::from(last);
    }
    if let Some(play_count) = fields.play_count {
        block[13] = play_count;
    }
    block
}

/// DES-CBC with PKCS7, keyed and initialised with the literal. The encrypted
/// block is 504 bytes and the trailer needs the last two, so the copy is
/// clipped rather than allowed to overrun.
pub fn encrypt_block(block: &[u8; COMMAND_SIZE]) -> [u8; PACKET_SIZE] {
    let sealed = EncryptorDes::new(&PACKET_KEY.into(), &PACKET_KEY.into())
        .encrypt_padded_vec_mut::<Pkcs7>(block);
    let mut packet = [0_u8; PACKET_SIZE];
    let copy = sealed.len().min(PACKET_SIZE - TRAILER.len());
    packet[..copy].copy_from_slice(&sealed[..copy]);
    packet[PACKET_SIZE - 2..].copy_from_slice(&TRAILER);
    packet
}

/// One command as the bytes to write: the frame alone, or the frame with an
/// image behind it in the **same** transfer.
///
/// One transfer is a device requirement rather than a convenience: sending the
/// frame and the image separately makes the panel accept a frame briefly and
/// then stop responding.
pub fn packet(opcode: u8, elapsed_ms: u32, fields: &Fields, payload: Option<&[u8]>) -> Vec<u8> {
    let frame = encrypt_block(&command_block(opcode, elapsed_ms, fields));
    let Some(payload) = payload.filter(|payload| !payload.is_empty()) else {
        return frame.to_vec();
    };
    let mut write = Vec::with_capacity(frame.len() + payload.len());
    write.extend_from_slice(&frame);
    write.extend_from_slice(payload);
    write
}

/// A command carrying one byte at offset 8.
pub fn parameter(opcode: u8, byte: u8, elapsed_ms: u32) -> Vec<u8> {
    packet(
        opcode,
        elapsed_ms,
        &Fields {
            parameter: Some(byte),
            ..Fields::default()
        },
        None,
    )
}

/// A command carrying an image, its length written big-endian at offset 8.
pub fn image(opcode: u8, payload: &[u8], elapsed_ms: u32) -> Vec<u8> {
    packet(
        opcode,
        elapsed_ms,
        &Fields {
            payload_length: Some(payload.len() as u32),
            ..Fields::default()
        },
        Some(payload),
    )
}

/// The plaintext of a frame, for tests that assert field placement rather than
/// reading opaque bytes back.
#[cfg(test)]
pub(crate) fn decrypt_block(packet: &[u8]) -> [u8; COMMAND_SIZE] {
    use cipher::BlockDecryptMut;
    // PKCS7 rounds the 500-byte block up to the next 8-byte multiple; the bytes
    // before the trailer hold that padding, not ciphertext.
    let sealed_len = COMMAND_SIZE.div_ceil(8) * 8;
    let sealed = packet[..sealed_len.min(packet.len())].to_vec();
    let plain = cbc::Decryptor::<Des>::new(&PACKET_KEY.into(), &PACKET_KEY.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&sealed)
        .expect("a decryptable block");
    let mut block = [0_u8; COMMAND_SIZE];
    block.copy_from_slice(&plain);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decrypt(packet: &[u8]) -> [u8; COMMAND_SIZE] {
        decrypt_block(packet)
    }

    #[test]
    fn a_command_block_puts_every_field_where_the_panel_reads_it() {
        let block = command_block(
            opcode::PUSH_JPEG,
            0x0102_0304,
            &Fields {
                payload_length: Some(0x0a0b_0c0d),
                ..Fields::default()
            },
        );
        assert_eq!(block[0], opcode::PUSH_JPEG);
        assert_eq!(block[1], 0);
        assert_eq!(&block[2..4], &MAGIC);
        // Little-endian timestamp, big-endian payload length.
        assert_eq!(&block[4..8], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&block[8..12], &[0x0a, 0x0b, 0x0c, 0x0d]);
        assert_eq!(block[12], 0);
        assert_eq!(block[13], 0);

        let parameter = command_block(
            opcode::BRIGHTNESS,
            0,
            &Fields {
                parameter: Some(70),
                ..Fields::default()
            },
        );
        assert_eq!(parameter[8], 70);
        assert_eq!(&parameter[9..12], &[0, 0, 0]);
    }

    #[test]
    fn a_frame_is_five_hundred_twelve_bytes_ending_in_the_trailer() {
        let frame = encrypt_block(&command_block(opcode::VERSION, 0, &Fields::default()));
        assert_eq!(frame.len(), PACKET_SIZE);
        assert_eq!(&frame[510..], &TRAILER);
        assert_eq!(decrypt(&frame)[0], opcode::VERSION);
    }

    #[test]
    fn an_image_follows_its_frame_in_one_write() {
        let jpeg = [0xff_u8, 0xd8, 0xff, 0xe0];
        let write = image(opcode::PUSH_JPEG, &jpeg, 7);
        assert_eq!(write.len(), PACKET_SIZE + jpeg.len());
        assert_eq!(&write[PACKET_SIZE..], &jpeg);
        let block = decrypt(&write);
        assert_eq!(block[0], opcode::PUSH_JPEG);
        assert_eq!(&block[8..12], &(jpeg.len() as u32).to_be_bytes());
    }

    #[test]
    fn a_parameter_command_carries_no_payload() {
        let write = parameter(opcode::ROTATE, 2, 0);
        assert_eq!(write.len(), PACKET_SIZE);
        assert_eq!(decrypt(&write)[8], 2);
    }

    #[test]
    fn the_same_command_twice_differs_only_in_its_timestamp() {
        let first = parameter(opcode::VERSION, 0, 100);
        let second = parameter(opcode::VERSION, 0, 200);
        assert_ne!(first, second);
        assert_eq!(&decrypt(&first)[..4], &decrypt(&second)[..4]);
        assert_eq!(&decrypt(&first)[8..], &decrypt(&second)[8..]);
    }
}
