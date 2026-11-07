use vodozemac::base64_decode;
use vrf_r255::{Proof, PublicKey};
use wasm_bindgen::prelude::*;

// Must match VRF_ALPHA_PREFIX in end2/src/directory.rs.
const VRF_ALPHA_PREFIX: &[u8] = b"end2/username/v1:";

/// Returns whether `proof_b64` proves that `index_hex` is the directory index that the
/// provider's VRF key assigns to `username`. `index_hex` must be lowercase hex.
#[wasm_bindgen]
#[must_use]
pub fn verify_directory_index(
    public_key_b64: &str,
    username: &str,
    index_hex: &str,
    proof_b64: &str,
) -> bool {
    vrf_output(public_key_b64, username, proof_b64)
        .is_some_and(|index| is_hex_of(index_hex, &index))
}

fn vrf_output(public_key_b64: &str, username: &str, proof_b64: &str) -> Option<[u8; 64]> {
    let public_key = PublicKey::from_bytes(base64_decode(public_key_b64).ok()?.try_into().ok()?)?;
    let proof = Proof::from_bytes(base64_decode(proof_b64).ok()?.try_into().ok()?)?;
    let alpha = [VRF_ALPHA_PREFIX, username.as_bytes()].concat();
    public_key.verify(&alpha, &proof).into()
}

// Exact lowercase match: validators key the store by this string, so "AB" and "ab" are different indexes.
fn is_hex_of(hex: &str, bytes: &[u8]) -> bool {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    hex.len() == bytes.len() * 2
        && hex
            .as_bytes()
            .chunks_exact(2)
            .zip(bytes)
            .all(|(pair, b)| pair == [DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 0xf)]])
}
