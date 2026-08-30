use thiserror::Error;

pub const COMMIT_MARKER_MAGIC: [u8; 8] = *b"DBLGCMT\0";
pub const COMMIT_MARKER_VERSION: u16 = 1;
pub const COMMIT_MARKER_LEN: usize = 32;
pub const APPEND_LOG_FORMAT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitMarker {
    pub generation_id: u64,
    pub log_format_version: u16,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommitMarkerError {
    #[error("marker generation id must be greater than zero")]
    ZeroGeneration,
    #[error("marker has {found} bytes, expected {expected}")]
    Length { found: usize, expected: usize },
    #[error("magic mismatch")]
    Magic,
    #[error("checksum mismatch: expected {expected:08x}, computed {computed:08x}")]
    Checksum { expected: u32, computed: u32 },
    #[error("unsupported marker version {found}; expected {expected}")]
    Version { found: u16, expected: u16 },
    #[error("invalid marker header length {0}")]
    HeaderLength(u16),
    #[error("marker generation {found} disagrees with filename generation {expected}")]
    GenerationMismatch { found: u64, expected: u64 },
    #[error("unsupported append-log format version {found}; expected {expected}")]
    LogFormat { found: u16, expected: u16 },
    #[error("marker reserved field is nonzero: {0:#06x}")]
    Reserved(u16),
    #[error("unsupported marker flags {0:#010x}")]
    Flags(u32),
}

pub fn encode_commit_marker(generation_id: u64) -> Result<[u8; COMMIT_MARKER_LEN], CommitMarkerError> {
    if generation_id == 0 {
        return Err(CommitMarkerError::ZeroGeneration);
    }

    let mut bytes = [0_u8; COMMIT_MARKER_LEN];
    bytes[..8].copy_from_slice(&COMMIT_MARKER_MAGIC);
    bytes[8..10].copy_from_slice(&COMMIT_MARKER_VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(COMMIT_MARKER_LEN as u16).to_le_bytes());
    bytes[12..20].copy_from_slice(&generation_id.to_le_bytes());
    bytes[20..22].copy_from_slice(&APPEND_LOG_FORMAT_VERSION.to_le_bytes());
    bytes[22..24].copy_from_slice(&0_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&0_u32.to_le_bytes());
    let checksum = crc32_ieee(&bytes[..28]);
    bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

pub fn decode_commit_marker(
    bytes: &[u8],
    filename_generation: u64,
) -> Result<CommitMarker, CommitMarkerError> {
    if filename_generation == 0 {
        return Err(CommitMarkerError::ZeroGeneration);
    }
    if bytes.len() != COMMIT_MARKER_LEN {
        return Err(CommitMarkerError::Length {
            found: bytes.len(),
            expected: COMMIT_MARKER_LEN,
        });
    }
    if bytes[..8] != COMMIT_MARKER_MAGIC {
        return Err(CommitMarkerError::Magic);
    }

    let expected_crc = read_u32(&bytes[28..32]);
    let computed_crc = crc32_ieee(&bytes[..28]);
    if expected_crc != computed_crc {
        return Err(CommitMarkerError::Checksum {
            expected: expected_crc,
            computed: computed_crc,
        });
    }

    let version = read_u16(&bytes[8..10]);
    if version != COMMIT_MARKER_VERSION {
        return Err(CommitMarkerError::Version {
            found: version,
            expected: COMMIT_MARKER_VERSION,
        });
    }
    let header_len = read_u16(&bytes[10..12]);
    if usize::from(header_len) != COMMIT_MARKER_LEN {
        return Err(CommitMarkerError::HeaderLength(header_len));
    }
    let generation_id = read_u64(&bytes[12..20]);
    if generation_id == 0 || generation_id != filename_generation {
        return Err(CommitMarkerError::GenerationMismatch {
            found: generation_id,
            expected: filename_generation,
        });
    }
    let log_format_version = read_u16(&bytes[20..22]);
    if log_format_version != APPEND_LOG_FORMAT_VERSION {
        return Err(CommitMarkerError::LogFormat {
            found: log_format_version,
            expected: APPEND_LOG_FORMAT_VERSION,
        });
    }
    let reserved = read_u16(&bytes[22..24]);
    if reserved != 0 {
        return Err(CommitMarkerError::Reserved(reserved));
    }
    let flags = read_u32(&bytes[24..28]);
    if flags != 0 {
        return Err(CommitMarkerError::Flags(flags));
    }

    Ok(CommitMarker {
        generation_id,
        log_format_version,
    })
}

fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_standard_check_vector() {
        assert_eq!(crc32_ieee(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn marker_round_trip_binds_generation() {
        let encoded = encode_commit_marker(42).expect("encode marker");
        let decoded = decode_commit_marker(&encoded, 42).expect("decode marker");
        assert_eq!(decoded.generation_id, 42);
        assert_eq!(decoded.log_format_version, APPEND_LOG_FORMAT_VERSION);
        assert!(decode_commit_marker(&encoded, 41).is_err());
    }

    #[test]
    fn marker_corruption_is_rejected() {
        let mut encoded = encode_commit_marker(7).expect("encode marker");
        encoded[12] ^= 0x80;
        assert!(matches!(
            decode_commit_marker(&encoded, 7),
            Err(CommitMarkerError::Checksum { .. })
        ));
    }
}
