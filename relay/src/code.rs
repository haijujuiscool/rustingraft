//! Room codes: 6 symbols from an alphabet without look-alikes (no 0/O, 1/I/L).

use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CODE_ALPHABET: &[u8; 31] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
pub const CODE_LEN: usize = 6;

pub fn generate_code() -> String {
    // Rejection sampling keeps every symbol equally likely (248 = 8 * 31).
    let n = CODE_ALPHABET.len();
    let limit = (256 / n * n) as u8;
    let mut code = String::with_capacity(CODE_LEN);
    while code.len() < CODE_LEN {
        for b in random_bytes() {
            if b < limit && code.len() < CODE_LEN {
                code.push(CODE_ALPHABET[b as usize % n] as char);
            }
        }
    }
    code
}

pub fn is_valid_code(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b))
}

/// 16 bytes from /dev/urandom, or where that does not exist, from SipHash with a random
/// std key over the time, a counter and a stack address.
fn random_bytes() -> [u8; 16] {
    let mut buf = [0u8; 16];
    if File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).is_ok() {
        return buf;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for chunk in buf.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_usize(&nanos as *const u128 as usize);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn alphabet_is_unambiguous_and_unique() {
        let set: HashSet<_> = CODE_ALPHABET.iter().collect();
        assert_eq!(set.len(), CODE_ALPHABET.len());
        for c in b"01ILO" {
            assert!(!set.contains(c));
        }
    }

    #[test]
    fn codes_are_valid_and_spread() {
        let codes: HashSet<String> = (0..2000).map(|_| generate_code()).collect();
        assert!(codes.len() > 1990);
        assert!(codes.iter().all(|c| is_valid_code(c)));
        let used: HashSet<u8> = codes.iter().flat_map(|c| c.bytes()).collect();
        assert_eq!(used.len(), CODE_ALPHABET.len());
    }

    #[test]
    fn validation() {
        assert!(is_valid_code("AB23CD"));
        for bad in ["", "AB23C", "AB23CDE", "ab23cd", "AB23C0", "AB 3CD", "\u{c4}B23C"] {
            assert!(!is_valid_code(bad), "{bad}");
        }
    }
}
