//! The two secrets the bridge hands out, and the shape of each.
//!
//! A pairing code is read aloud or copied by a person, so it is four letters
//! and the port it belongs to. An omniauth is only ever handled by a program,
//! so it is long enough that guessing it is not a strategy.

use uuid::Uuid;

/// No `I` and no `O`: a code gets retyped from a terminal into a browser, and
/// those two are the letters that come back as `1` and `0`.
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";

/// How many letters lead a pairing code. Four letters over this alphabet is
/// 331,776 codes, which only means anything alongside the attempt limit in
/// [`crate::state`]; on its own it would be guessable in an afternoon.
pub const CODE_LETTERS: usize = 4;

/// Cryptographically seeded bytes, borrowed from the same source `uuid` uses
/// for a v4. Taking them this way keeps the dependency list at what the
/// workspace already builds.
pub fn random(count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        out.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    out.truncate(count);
    out
}

/// `XULA-1998` — four letters and the port they were minted on.
///
/// The port travels inside the code on purpose. It is the one thing a website
/// cannot discover without knocking on ten doors, and a person copying the
/// code across is already carrying it.
pub fn pairing_code(port: u16) -> String {
    /* 256 is not a multiple of 24, so folding a whole byte would make the
       first eight letters come up slightly more often than the rest. 240 is,
       so bytes at or above it are thrown away and another one is drawn. */
    const CEILING: u8 = 240;
    let mut letters = String::with_capacity(CODE_LETTERS);
    while letters.len() < CODE_LETTERS {
        for byte in random(CODE_LETTERS * 2) {
            if byte >= CEILING {
                continue;
            }
            letters.push(ALPHABET[byte as usize % ALPHABET.len()] as char);
            if letters.len() == CODE_LETTERS {
                break;
            }
        }
    }
    format!("{letters}-{port}")
}

/// Split a typed code into its letters and the port it claims.
pub fn read_code(code: &str) -> Option<(String, u16)> {
    let cleaned: String = code
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    let (letters, port) = cleaned.split_once('-')?;
    if letters.len() != CODE_LETTERS {
        return None;
    }
    let letters = letters.to_ascii_uppercase();
    if !letters.bytes().all(|byte| ALPHABET.contains(&byte)) {
        return None;
    }
    Some((letters, port.parse().ok()?))
}

/// The token a website holds afterwards. 24 bytes of entropy, prefixed so it
/// is obvious in a log what somebody just pasted somewhere public.
pub fn omniauth() -> String {
    format!("omniauth_{}", crate::sha256::hex(&random(24)))
}

/// The control token, which is not handed to anybody: it lives in a 0600 file
/// and its only job is to prove that the caller could read that file.
pub fn control() -> String {
    format!("omnictl_{}", crate::sha256::hex(&random(24)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pairing_code_is_four_letters_and_its_port() {
        let code = pairing_code(1998);
        assert_eq!(code.len(), 9, "{code}");
        assert!(code.ends_with("-1998"), "{code}");
        let (letters, port) = read_code(&code).expect("its own code reads back");
        assert_eq!(port, 1998);
        assert_eq!(letters.len(), CODE_LETTERS);
    }

    #[test]
    fn a_code_is_read_the_way_a_person_would_have_typed_it() {
        assert_eq!(read_code("xula-1998"), Some(("XULA".into(), 1998)));
        assert_eq!(read_code(" XULA-1998 "), Some(("XULA".into(), 1998)));
        assert_eq!(read_code("XULA-1998"), Some(("XULA".into(), 1998)));
    }

    #[test]
    fn a_code_that_is_not_one_is_refused_rather_than_guessed_at() {
        assert_eq!(read_code("XULA1998"), None, "no separator");
        assert_eq!(read_code("XUL-1998"), None, "three letters");
        assert_eq!(read_code("XULI-1998"), None, "I is not in the alphabet");
        assert_eq!(read_code("XULO-1998"), None, "nor is O");
        assert_eq!(read_code("XULA-nope"), None, "not a port");
    }

    #[test]
    fn the_letters_cover_the_alphabet_rather_than_a_corner_of_it() {
        let seen: std::collections::HashSet<char> = (0..500)
            .flat_map(|_| pairing_code(1998).chars().take(CODE_LETTERS).collect::<Vec<_>>())
            .collect();
        assert!(seen.len() > 20, "only saw {} distinct letters", seen.len());
    }

    #[test]
    fn two_tokens_are_never_the_same_one() {
        let first = omniauth();
        assert!(first.starts_with("omniauth_"));
        assert_eq!(first.len(), "omniauth_".len() + 48);
        assert_ne!(first, omniauth());
    }
}
