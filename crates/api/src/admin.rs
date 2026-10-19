//! Otorisasi admin (D4b, dipakai ulang di D6): satu token rahasia di
//! `ADMIN_TOKEN`, dikirim sebagai `Authorization: Bearer <token>`.
//!
//! - Gagal tertutup: token tidak diisi atau terlalu pendek (< 32 karakter)
//!   berarti **tidak ada** yang bisa masuk, bukan "semua boleh".
//! - Perbandingan tidak bocor lewat waktu: kedua sisi di-hash dulu (panjangnya
//!   sama), lalu dibandingkan tanpa berhenti di byte pertama yang berbeda.
//! - Tidak untuk browser: CORS tidak mengizinkan header `Authorization` dan
//!   metode `DELETE`, jadi halaman web mana pun tidak bisa memanggilnya. Alatnya
//!   `curl` (lihat README).

use sha2::{Digest, Sha256};

use crate::config::Secret;

/// Panjang minimum token. `openssl rand -hex 32` menghasilkan 64.
pub const MIN_TOKEN_CHARS: usize = 32;

/// Isi header `Authorization` -> token bila skemanya `Bearer` (huruf besar/kecil
/// tidak dibedakan, sesuai RFC 7235).
fn bearer(header: Option<&str>) -> Option<&str> {
    let value = header?.trim();
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
        .filter(|t| !t.is_empty())
}

fn digest(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

/// `true` hanya jika admin diaktifkan dan tokennya cocok persis.
pub fn authorized(configured: &Secret, authorization: Option<&str>) -> bool {
    let expected = configured.expose();
    if expected.chars().count() < MIN_TOKEN_CHARS {
        return false;
    }
    let Some(given) = bearer(authorization) else {
        return false;
    };
    let (a, b) = (digest(expected), digest(given));
    a.iter().zip(&b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

    fn secret(s: &str) -> Secret {
        Secret::new(s)
    }

    #[test]
    fn the_exact_token_is_accepted_in_any_bearer_casing() {
        let t = secret(TOKEN);
        assert!(authorized(&t, Some(&format!("Bearer {TOKEN}"))));
        assert!(authorized(&t, Some(&format!("bearer {TOKEN}"))));
        assert!(authorized(&t, Some(&format!("  BEARER   {TOKEN}  "))));
    }

    #[test]
    fn anything_else_is_refused() {
        let t = secret(TOKEN);
        for bad in [
            None,
            Some(""),
            Some("Bearer"),
            Some("Bearer "),
            Some(TOKEN),
            Some(&format!("Basic {TOKEN}")[..]),
            Some("Bearer wrong"),
            Some(&format!("Bearer {TOKEN}x")[..]),
            Some(&format!("Bearer {}", &TOKEN[1..])[..]),
        ] {
            assert!(!authorized(&t, bad), "{bad:?}");
        }
    }

    #[test]
    fn without_a_long_enough_token_nobody_gets_in_not_even_with_the_same_text() {
        for configured in ["", "   ", "short", &"x".repeat(MIN_TOKEN_CHARS - 1)] {
            let t = secret(configured);
            assert!(!authorized(&t, Some(&format!("Bearer {configured}"))));
            assert!(!authorized(&t, Some("Bearer ")));
            assert!(!authorized(&t, None));
        }
        let minimum = "x".repeat(MIN_TOKEN_CHARS);
        assert!(authorized(
            &secret(&minimum),
            Some(&format!("Bearer {minimum}"))
        ));
    }
}
