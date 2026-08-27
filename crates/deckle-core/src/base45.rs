//! Base45 (RFC 9285).
//!
//! Its alphabet is exactly QR's alphanumeric character set, so a Base45 payload
//! encodes in alphanumeric mode at 5.5 bits per character. Against byte mode
//! that costs about 3%, and it buys the property the bootstrap page exists for:
//! any commodity QR reader shows the content as text a person can see and copy,
//! rather than as binary it will refuse to display.

const ALPHABET: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 3 / 2 + 2);
    let mut chunks = data.chunks_exact(2);
    for c in chunks.by_ref() {
        let n = (c[0] as usize) << 8 | c[1] as usize;
        out.push(ALPHABET[n % 45] as char);
        out.push(ALPHABET[(n / 45) % 45] as char);
        out.push(ALPHABET[n / 45 / 45] as char);
    }
    if let [b] = chunks.remainder() {
        let n = *b as usize;
        out.push(ALPHABET[n % 45] as char);
        out.push(ALPHABET[n / 45] as char);
    }
    out
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let vals: Option<Vec<usize>> = text
        .bytes()
        .map(|b| ALPHABET.iter().position(|&a| a == b))
        .collect();
    let vals = vals?;
    let mut out = Vec::with_capacity(vals.len() * 2 / 3 + 1);
    let mut i = 0;
    while i + 3 <= vals.len() {
        let n = vals[i] + vals[i + 1] * 45 + vals[i + 2] * 45 * 45;
        if n > 0xFFFF {
            return None;
        }
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 3;
    }
    match vals.len() - i {
        0 => Some(out),
        2 => {
            let n = vals[i] + vals[i + 1] * 45;
            if n > 0xFF {
                return None;
            }
            out.push(n as u8);
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_examples() {
        assert_eq!(encode(b"AB"), "BB8");
        assert_eq!(encode(b"Hello!!"), "%69 VD92EX0");
        assert_eq!(encode(b"base-45"), "UJCLQE7W581");
        assert_eq!(decode("QED8WEX0").unwrap(), b"ietf!");
    }

    #[test]
    fn round_trips_arbitrary_bytes() {
        let mut r = crate::rng::Rng::new(4);
        for len in [0usize, 1, 2, 3, 255, 1000, 4097] {
            let data: Vec<u8> = (0..len).map(|_| r.next_u32() as u8).collect();
            let text = encode(&data);
            assert!(text.bytes().all(|b| ALPHABET.contains(&b)), "len {len}");
            assert_eq!(decode(&text).unwrap(), data, "len {len}");
        }
    }
}
