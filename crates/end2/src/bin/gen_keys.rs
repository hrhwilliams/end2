//! Prints a fresh provider signing key pair and VRF directory key pair as `.env` lines:
//! `SERVER_SIGNING_KEY`, `SERVER_VERIFYING_KEY`, `VRF_SECRET_KEY` and `VITE_VRF_PUBLIC_KEY`.
//! `setup.sh` runs it inside the backend image.

use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};
use ed25519_dalek::SigningKey;
use end2::DirectoryKey;
use rand_core::{OsRng, RngCore};

fn main() {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing = SigningKey::from_bytes(&seed);
    let vrf = DirectoryKey::generate();

    println!(
        "SERVER_SIGNING_KEY={}",
        BASE64_STANDARD_NO_PAD.encode(signing.to_bytes())
    );
    println!(
        "SERVER_VERIFYING_KEY={}",
        BASE64_STANDARD_NO_PAD.encode(signing.verifying_key().to_bytes())
    );
    println!("VRF_SECRET_KEY={}", vrf.secret_base64());
    println!("VITE_VRF_PUBLIC_KEY={}", vrf.public_key_base64());
}
