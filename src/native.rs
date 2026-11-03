//! Locate the terrain commit by code rather than executable version or RVA.
use enshwick::{Load, Off};

// Function entry, argument spills, local workspace, and Words::count access.
// Relative branch displacement is deliberately excluded. Verified unique in
// both archived age-399 and current age-479 dedicated-server executables.
pub const COMMIT_PATTERN: &str = "4C 8B DC 4D 89 4B 20 4D 89 43 18 49 89 53 10 49 89 4B 08 41 54 48 81 EC C0 04 00 00 45 33 E4 49 8B C1 45 39 60 08 0F 86";

pub fn commit(load: &Load<'_>) -> Result<u64, Off> {
    load.find_code(COMMIT_PATTERN).map_err(|e| {
        Off::new(format!(
            "Terrain commit signature unavailable or ambiguous; refusing capture/replay: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use enshwick::code::{FindError, Pattern};

    #[test]
    fn commit_signature_follows_relocation_and_rejects_missing_or_ambiguous_code() {
        let pattern = Pattern::parse(COMMIT_PATTERN).unwrap();
        let bytes: Vec<u8> = COMMIT_PATTERN
            .split_whitespace()
            .map(|v| u8::from_str_radix(v, 16).unwrap())
            .collect();
        let mut code = vec![0x90; 73];
        code.extend_from_slice(&bytes);
        assert_eq!(pattern.find_unique(&code), Ok(73));
        code.extend_from_slice(&bytes);
        assert_eq!(pattern.find_unique(&code), Err(FindError::Many));
        assert_eq!(pattern.find_unique(&[0x90; 100]), Err(FindError::None));
    }
}
