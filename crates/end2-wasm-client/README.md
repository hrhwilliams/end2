# End2 WASM client

Web client for interacting with the End2 end-to-end encrypted messaging service.

## Building

Run the following command to build for webpack:

```
wasm-pack build --target bundler
```

And then serve using npm:

```
cd frontend
npm run dev
```

## Reading keys from the CometBFT validators

Set two variables when you build the frontend. Vite reads them at build time, so
both values are pinned in the bundle and the client never fetches them from the
provider.

- `VITE_VRF_PUBLIC_KEY` is the provider's VRF public key, unpadded base64.
  `cargo run -p end2 --bin gen_vrf_key` prints it next to the matching
  `VRF_SECRET_KEY` for the backend.
- `VITE_COMET_RPC_URLS` is a comma-separated list of validator RPC base URLs,
  such as
  `https://chat.fiatlux.dev/comet/0,https://chat.fiatlux.dev/comet/1,https://chat.fiatlux.dev/comet/2,https://chat.fiatlux.dev/comet/3`.

With both set, the client checks the VRF proof that binds each user's directory
index to their username. It then reads device keys from every listed validator and
accepts a value only when at least f+1 validators return identical bytes, where
f = floor((n-1)/3). If the proof fails or no quorum forms, the lookup fails. If
`VITE_COMET_RPC_URLS` is unset, the client asks the provider for keys, as the
Postgres and Ethereum controls expect.

In the testbed, the `/comet/<n>/` proxies in `config/nginx/locations.conf` run on the
provider's host, so they give no independence from the provider. A real deployment
points each URL at a validator that a separate party operates.
