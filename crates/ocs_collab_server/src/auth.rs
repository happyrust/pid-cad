//! Pure-Rust credential hashing and verification (PBKDF2-HMAC-SHA256) for #68.
//!
//! Zero external dependencies so `--offline --locked` builds stay self-contained.
//! Credentials are never persisted in plaintext and never logged: SQLite stores
//! only `pbkdf2_sha256$<iterations>$<salt_hex>$<hash_hex>`.

const ITERATIONS: u32 = 10_000;
const PREFIX: &str = "pbkdf2_sha256";

/// Hashes a plaintext credential with a fresh 128-bit random salt.
pub fn hash_credential(secret: &str) -> String {
    let salt = *uuid::Uuid::new_v4().as_bytes();
    hash_with_salt(secret.as_bytes(), &salt, ITERATIONS)
}

/// Verifies `secret` against a stored `pbkdf2_sha256$<iter>$<salt>$<dk>` hash
/// using constant-time equality.
pub fn verify_credential(secret: &str, stored: &str) -> bool {
    if secret.is_empty() || stored.is_empty() {
        return false;
    }
    let mut parts = stored.split('$');
    let (Some(prefix), Some(iter_s), Some(salt_hex), Some(dk_hex), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    if prefix != PREFIX {
        return false;
    }
    let Ok(iterations) = iter_s.parse::<u32>() else {
        return false;
    };
    if iterations == 0 {
        return false;
    }
    let (Some(salt), Some(expected)) = (hex_decode(salt_hex), hex_decode(dk_hex)) else {
        return false;
    };
    if expected.len() != 32 {
        return false;
    }
    let derived = pbkdf2_hmac_sha256(secret.as_bytes(), &salt, iterations);
    ct_eq(&derived, &expected)
}

fn hash_with_salt(password: &[u8], salt: &[u8], iterations: u32) -> String {
    let dk = pbkdf2_hmac_sha256(password, salt, iterations);
    format!(
        "{PREFIX}${iterations}${}${}",
        hex_encode(salt),
        hex_encode(&dk)
    )
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = hex.as_bytes();
    let nibble = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.as_chunks::<2>().0 {
        let hi = nibble(chunk[0])?;
        let lo = nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

/// Decodes standard Base64 (used for `Authorization: Basic ...`).
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if !cleaned.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity((cleaned.len() / 4) * 3);
    for chunk in cleaned.as_chunks::<4>().0 {
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            let c = val(chunk[2])?;
            out.push((b << 4) | (c >> 2));
            if chunk[3] != b'=' {
                let d = val(chunk[3])?;
                out.push((c << 6) | d);
            }
        }
    }
    Some(out)
}

/// PBKDF2-HMAC-SHA256 producing a single 32-byte derived key block (RFC 2898).
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut salt_block = Vec::with_capacity(salt.len() + 4);
    salt_block.extend_from_slice(salt);
    salt_block.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &salt_block);
    let mut out = u;
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for (o, b) in out.iter_mut().zip(u.iter()) {
            *o ^= *b;
        }
    }
    out
}

/// HMAC-SHA256 (RFC 2104).
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Vec::with_capacity(64 + data.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(data);
    let inner_hash = sha256(&inner);

    let mut outer = [0u8; 96];
    outer[..64].copy_from_slice(&opad);
    outer[64..].copy_from_slice(&inner_hash);
    sha256(&outer)
}

/// Standard FIPS 180-4 SHA-256.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut digest = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_standard_test_vector() {
        let digest = hex_encode(&sha256(b"abc"));
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_and_verify_credential_round_trip() {
        let hash = hash_credential("s3cr3t-token!");
        assert!(hash.starts_with("pbkdf2_sha256$10000$"));
        assert!(!hash.contains("s3cr3t-token!"));
        assert!(verify_credential("s3cr3t-token!", &hash));
        assert!(!verify_credential("wrong-token", &hash));
        assert!(!verify_credential("", &hash));
        assert!(!verify_credential("s3cr3t-token!", ""));
    }

    #[test]
    fn base64_decode_decodes_basic_auth_token() {
        let decoded = base64_decode("dTE6c2VjcmV0").expect("valid base64");
        assert_eq!(String::from_utf8(decoded).expect("utf8"), "u1:secret");
    }
}
