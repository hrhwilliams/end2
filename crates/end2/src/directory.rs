use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};
use rand_core::OsRng;
use serde::{Serialize, Serializer};
use vrf_r255::{PublicKey, SecretKey};

use crate::serialize_as_base64;

// Must match VRF_ALPHA_PREFIX in end2-wasm-client/src/directory.rs.
const VRF_ALPHA_PREFIX: &[u8] = b"end2/username/v1:";

fn alpha(username: &str) -> Vec<u8> {
    [VRF_ALPHA_PREFIX, username.as_bytes()].concat()
}

fn serialize_as_hex<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex::encode(bytes))
}

/// A user's key-directory index and the VRF proof that binds it to their username.
#[derive(Clone, Debug, Serialize)]
pub struct DirectoryIndex {
    #[serde(serialize_with = "serialize_as_hex")]
    pub index: [u8; 64],
    #[serde(serialize_with = "serialize_as_base64")]
    pub proof: [u8; 80],
}

/// The provider's VRF secret key. Each username maps to exactly one directory index.
pub struct DirectoryKey {
    secret: SecretKey,
}

impl DirectoryKey {
    #[must_use]
    pub fn generate() -> Self {
        Self {
            secret: SecretKey::generate(OsRng),
        }
    }

    pub fn from_base64(secret_b64: &str) -> Result<Self, &'static str> {
        let bytes: [u8; 32] = BASE64_STANDARD_NO_PAD
            .decode(secret_b64)
            .map_err(|_| "VRF secret key is not unpadded base64")?
            .try_into()
            .map_err(|_| "VRF secret key must be 32 bytes")?;
        Option::from(SecretKey::from_bytes(bytes))
            .map(|secret| Self { secret })
            .ok_or("VRF secret key is not a canonical ristretto255 scalar")
    }

    #[must_use]
    pub fn secret_base64(&self) -> String {
        BASE64_STANDARD_NO_PAD.encode(self.secret.to_bytes())
    }

    #[must_use]
    pub fn public_key_base64(&self) -> String {
        BASE64_STANDARD_NO_PAD.encode(PublicKey::from(self.secret).to_bytes())
    }

    #[must_use]
    pub fn index_for(&self, username: &str) -> DirectoryIndex {
        let alpha = alpha(username);
        let proof = self.secret.prove(&alpha);
        // vrf-r255 exposes the output only through verify.
        let index = Option::from(PublicKey::from(self.secret).verify(&alpha, &proof))
            .expect("a proof from our own key verifies");
        DirectoryIndex {
            index,
            proof: proof.to_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrf_r255::Proof;

    fn verify(public_key_b64: &str, username: &str, proof: [u8; 80]) -> Option<[u8; 64]> {
        let pk: [u8; 32] = BASE64_STANDARD_NO_PAD
            .decode(public_key_b64)
            .expect("base64 public key")
            .try_into()
            .expect("32-byte public key");
        let pk = PublicKey::from_bytes(pk).expect("valid public key");
        let proof = Proof::from_bytes(proof).expect("valid proof encoding");
        pk.verify(&alpha(username), &proof).into()
    }

    #[test]
    fn index_verifies_under_public_key_and_is_unique_per_username() {
        let key = DirectoryKey::from_base64(&DirectoryKey::generate().secret_base64())
            .expect("secret key round-trips through base64");
        let pk = key.public_key_base64();

        let alice = key.index_for("alice");
        let bob = key.index_for("bob");

        assert_eq!(verify(&pk, "alice", alice.proof), Some(alice.index));
        assert_eq!(verify(&pk, "bob", bob.proof), Some(bob.index));
        assert_eq!(verify(&pk, "bob", alice.proof), None);
        assert_ne!(alice.index, bob.index);
        assert_eq!(key.index_for("alice").index, alice.index);
    }
}
