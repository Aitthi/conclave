//! Canonical codec for memory embeddings.
//!
//! Vectors are stored as contiguous little-endian IEEE-754 `f32` values. The
//! caller supplies the expected dimension when decoding so a truncated,
//! oversized, or dimension-mismatched BLOB fails loudly.

use crate::engine::error::AppError;

const F32_BYTES: usize = std::mem::size_of::<f32>();

/// Encode a vector as contiguous little-endian `f32` bytes.
pub fn encode(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * F32_BYTES);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decode exactly `dimension` little-endian `f32` values.
///
/// A BLOB whose byte length does not exactly match the expected dimension is
/// invalid; accepting it would let search compare vectors from different
/// spaces or silently ignore corrupt bytes.
pub fn decode(bytes: &[u8], dimension: usize) -> Result<Vec<f32>, AppError> {
    let expected_len = dimension.checked_mul(F32_BYTES).ok_or_else(|| {
        AppError::Invalid(format!(
            "embedding dimension {dimension} overflows the vector byte length"
        ))
    })?;

    if bytes.len() != expected_len {
        return Err(AppError::Invalid(format!(
            "embedding BLOB length mismatch: expected {expected_len} bytes for dimension \
             {dimension}, got {}",
            bytes.len()
        )));
    }

    Ok(bytes
        .chunks_exact(F32_BYTES)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_f32_bits() {
        let vector = [0.0, -1.25, f32::MIN_POSITIVE, f32::MAX, f32::NAN];
        let decoded = decode(&encode(&vector), vector.len()).expect("decode");

        let original_bits: Vec<u32> = vector.iter().map(|value| value.to_bits()).collect();
        let decoded_bits: Vec<u32> = decoded.iter().map(|value| value.to_bits()).collect();
        assert_eq!(decoded_bits, original_bits);
    }

    #[test]
    fn encoding_is_little_endian() {
        assert_eq!(encode(&[1.0]), 1.0f32.to_le_bytes());
    }

    #[test]
    fn wrong_length_is_invalid() {
        let error = decode(&[0; 7], 2).expect_err("seven bytes cannot hold two f32 values");
        assert!(
            matches!(error, AppError::Invalid(ref message) if message.contains("expected 8 bytes")),
            "unexpected error: {error}"
        );
    }
}
