//! Prints a fresh VRF directory key pair.
//!
//! `gen_vrf_key` prints `VRF_SECRET_KEY` and `VITE_VRF_PUBLIC_KEY` lines for `.env`.
//! `gen_vrf_key --sample <username>` prints a JSON fixture instead: the public key,
//! the username, and that username's `DirectoryIndex` under the fresh key.

use end2::DirectoryKey;

fn main() {
    let key = DirectoryKey::generate();
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.as_slice() {
        [] => {
            println!("VRF_SECRET_KEY={}", key.secret_base64());
            println!("VITE_VRF_PUBLIC_KEY={}", key.public_key_base64());
        }
        [flag, username] if flag == "--sample" => {
            let mut fixture =
                serde_json::to_value(key.index_for(username)).expect("DirectoryIndex serializes");
            fixture["public_key"] = key.public_key_base64().into();
            fixture["username"] = username.as_str().into();
            println!("{fixture}");
        }
        _ => {
            eprintln!("usage: gen_vrf_key [--sample <username>]");
            std::process::exit(2);
        }
    }
}
