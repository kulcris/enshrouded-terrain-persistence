//! Compact deterministic encoding for a 512-word terrain brick.

use std::io;

const WORDS: usize = 512;
const OMIT: u16 = 0xffff;
const RAW: u8 = 0;
const SPARSE: u8 = 1;
const BITMASK: u8 = 2;
const RLE: u8 = 3;

/// Encodes a brick using the smallest supported representation.
pub fn encode(words: &[u16; WORDS]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(1 + WORDS * 2);
    raw.push(RAW);
    for word in words {
        raw.extend_from_slice(&word.to_le_bytes());
    }

    let count = words.iter().filter(|&&word| word != OMIT).count();
    let mut sparse = Vec::with_capacity(3 + count * 4);
    sparse.push(SPARSE);
    sparse.extend_from_slice(&(count as u16).to_le_bytes());
    for (index, word) in words.iter().enumerate() {
        if *word != OMIT {
            sparse.extend_from_slice(&(index as u16).to_le_bytes());
            sparse.extend_from_slice(&word.to_le_bytes());
        }
    }

    let mut bitmask = Vec::with_capacity(1 + WORDS / 8 + count * 2);
    bitmask.push(BITMASK);
    bitmask.resize(1 + WORDS / 8, 0);
    for (index, word) in words.iter().enumerate() {
        if *word != OMIT {
            bitmask[1 + index / 8] |= 1 << (index % 8);
            bitmask.extend_from_slice(&word.to_le_bytes());
        }
    }

    let mut rle = Vec::with_capacity(1 + WORDS * 4);
    rle.push(RLE);
    let mut start = 0;
    while start < WORDS {
        let value = words[start];
        let mut end = start + 1;
        while end < WORDS && words[end] == value {
            end += 1;
        }
        rle.extend_from_slice(&((end - start) as u16).to_le_bytes());
        rle.extend_from_slice(&value.to_le_bytes());
        start = end;
    }

    [raw, sparse, bitmask, rle]
        .into_iter()
        .min_by_key(|candidate| (candidate.len(), candidate[0]))
        .expect("four encoding candidates")
}

/// Decodes a brick, rejecting malformed or non-exact representations.
pub fn decode(bytes: &[u8]) -> io::Result<[u16; WORDS]> {
    let Some(&tag) = bytes.first() else {
        return Err(invalid("empty brick encoding"));
    };
    match tag {
        RAW => decode_raw(bytes),
        SPARSE => decode_sparse(bytes),
        BITMASK => decode_bitmask(bytes),
        RLE => decode_rle(bytes),
        _ => Err(invalid("unknown brick encoding")),
    }
}

fn decode_raw(bytes: &[u8]) -> io::Result<[u16; WORDS]> {
    if bytes.len() != 1 + WORDS * 2 {
        return Err(invalid("invalid raw brick length"));
    }
    let mut words = [OMIT; WORDS];
    for (i, word) in words.iter_mut().enumerate() {
        *word = u16::from_le_bytes([bytes[1 + i * 2], bytes[2 + i * 2]]);
    }
    Ok(words)
}

fn decode_sparse(bytes: &[u8]) -> io::Result<[u16; WORDS]> {
    if bytes.len() < 3 {
        return Err(invalid("truncated sparse brick header"));
    }
    let count = usize::from(u16::from_le_bytes([bytes[1], bytes[2]]));
    if count > WORDS || bytes.len() != 3 + count * 4 {
        return Err(invalid("invalid sparse brick length"));
    }
    let mut words = [OMIT; WORDS];
    let mut previous = None;
    for entry in 0..count {
        let at = 3 + entry * 4;
        let index = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
        if index >= WORDS || previous.is_some_and(|last| index <= last) {
            return Err(invalid("sparse indices must be unique and increasing"));
        }
        let value = u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]);
        if value == OMIT {
            return Err(invalid("sparse entry cannot encode OMIT"));
        }
        words[index] = value;
        previous = Some(index);
    }
    Ok(words)
}

fn decode_bitmask(bytes: &[u8]) -> io::Result<[u16; WORDS]> {
    const HEADER: usize = 1 + WORDS / 8;
    if bytes.len() < HEADER {
        return Err(invalid("truncated bitmask brick header"));
    }
    let count = (0..WORDS).filter(|index| bit_is_set(bytes, *index)).count();
    if bytes.len() != HEADER + count * 2 {
        return Err(invalid("invalid bitmask brick length"));
    }
    let mut words = [OMIT; WORDS];
    let mut at = HEADER;
    for (index, word) in words.iter_mut().enumerate() {
        if bit_is_set(bytes, index) {
            let value = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            if value == OMIT {
                return Err(invalid("bitmask entry cannot encode OMIT"));
            }
            *word = value;
            at += 2;
        }
    }
    Ok(words)
}

fn bit_is_set(bytes: &[u8], index: usize) -> bool {
    bytes[1 + index / 8] & (1 << (index % 8)) != 0
}

fn decode_rle(bytes: &[u8]) -> io::Result<[u16; WORDS]> {
    if bytes.len() < 5 || (bytes.len() - 1) % 4 != 0 {
        return Err(invalid("invalid RLE brick length"));
    }
    let mut words = [OMIT; WORDS];
    let mut filled = 0usize;
    for chunk in bytes[1..].chunks_exact(4) {
        let run = usize::from(u16::from_le_bytes([chunk[0], chunk[1]]));
        if run == 0 || filled.checked_add(run).is_none_or(|end| end > WORDS) {
            return Err(invalid("invalid RLE run length"));
        }
        let value = u16::from_le_bytes([chunk[2], chunk[3]]);
        words[filled..filled + run].fill(value);
        filled += run;
    }
    if filled != WORDS {
        return Err(invalid("RLE runs do not fill the brick"));
    }
    Ok(words)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_dense_brick() {
        let words = std::array::from_fn(|index| (index as u16).wrapping_mul(31));
        let encoded = encode(&words);
        assert_eq!(encoded[0], RAW);
        assert_eq!(decode(&encoded).unwrap(), words);
    }

    #[test]
    fn round_trips_sparse_holes_compactly() {
        let mut words = [OMIT; WORDS];
        words[3] = 7;
        words[200] = 0x1200;
        words[511] = 9;
        let encoded = encode(&words);
        assert_eq!(encoded[0], SPARSE);
        assert_eq!(encoded.len(), 3 + 3 * 4);
        assert_eq!(decode(&encoded).unwrap(), words);
    }

    #[test]
    fn round_trips_uniform_brick_compactly() {
        let words = [42; WORDS];
        let encoded = encode(&words);
        assert_eq!(encoded[0], RLE);
        assert_eq!(encoded.len(), 5);
        assert_eq!(decode(&encoded).unwrap(), words);
    }

    #[test]
    fn bitmask_is_selected_for_middle_density() {
        let words = std::array::from_fn(|index| if index % 2 == 0 { index as u16 } else { OMIT });
        let encoded = encode(&words);
        assert_eq!(encoded[0], BITMASK);
        assert_eq!(decode(&encoded).unwrap(), words);
    }

    #[test]
    fn rejects_malformed_encodings() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[9]).is_err());
        assert!(decode(&[RAW, 0]).is_err());

        let mut duplicate = vec![SPARSE, 2, 0, 1, 0, 7, 0, 1, 0, 8, 0];
        assert!(decode(&duplicate).is_err());
        duplicate[7] = 0;
        assert!(decode(&duplicate).is_err()); // Out of order.

        assert!(decode(&[RLE, 0, 0, 3, 0]).is_err()); // Zero run.
        let mut overflow = vec![RLE];
        overflow.extend_from_slice(&513u16.to_le_bytes());
        overflow.extend_from_slice(&4u16.to_le_bytes());
        assert!(decode(&overflow).is_err());
        assert!(decode(&[RLE, 1, 0, 4, 0, 1]).is_err()); // Trailing byte.
    }
}
