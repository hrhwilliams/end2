# End2

An experimental user-auditable and misbehavior-resistant E2EE messaging application. End2
breaks down end-to-end messaging into three parts: entity authentication, key
distribution, and message relay.

### Entity authentication and message relay

There is one centralized service ([crates/end2](crates/end2)) that distributes the chat client ([crates/end2-wasm-client](crates/end2-wasm-client)) and handles both user authentication and message relay. Users are able
to create native accounts or create accounts via OAuth with their Discord or
Google accounts.

### Key distribution

Cryptographic material for establishing E2EE sessions is uploaded to and distributed
by third-party nodes running a private blockchain (CometBFT) to prevent service-mediated
equivocation of user's public keys. The repository currently has a Docker compose file
that starts four [CometBFT](https://github.com/cometbft/cometbft) and ABCI application nodes within the same network, but in practice these nodes would be independent.

#### Verifiable random functions

To prevent the service from equivocating the mapping from username to directory
index, usernames are mapped to indices via a verifiable random function (VRF) hash.
This is preferable to a SHA256 hash as it prevents dictionary attacks, while
remaining checkable by third parties wishing to verify that a given username
truly maps to the given hash. This VRF is provided by the [vrf-r255](https://docs.rs/vrf-r255/latest/vrf_r255/) crate.

## Repository map

### End2

|crate|path|description|
|-----|---|----------|
|end2 |[crates/end2](crates/end2)|Backend|
|end2-wasm-client|[crates/end2-wasm-client](crates/end2-wasm-client)|Frontend and client-side encryption library|
|end2-cometbft|[crates/end2-cometbft](crates/end2-cometbft)|CometBFT ABCI client|

### Correctness proof

The file [rocq/directory.v](rocq/directory.v) contains a correctness proof
of the directory written in Rocq. The proofs hold under cryptographic assumptions
such as collision resistance for keys and user identity hashes.

## Quick start

After cloning, run the included setup script to initialize the CometBFT node's
data and some secret keys for the backend.

```sh
./setup.sh
docker compose up --build
```
