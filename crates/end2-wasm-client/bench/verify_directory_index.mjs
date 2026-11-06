// Times verify_directory_index in the Node build of the wasm client.
//
// From crates/end2-wasm-client:
//   wasm-pack build --target nodejs --out-dir bench/pkg-node
//   node bench/verify_directory_index.mjs [iterations]
//
// bench/fixture.json comes from `cargo run -p end2 --bin gen_vrf_key -- --sample <username>`.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { cpus } from "node:os";

const iterations = Number(process.argv[2] ?? 10_000);
const require = createRequire(import.meta.url);
const { verify_directory_index } = require("./pkg-node/end2_wasm_client.js");
const fixture = JSON.parse(
  readFileSync(new URL("./fixture.json", import.meta.url), "utf8"),
);

const verify = (username = fixture.username, index = fixture.index) =>
  verify_directory_index(fixture.public_key, username, index, fixture.proof);

assert.equal(verify(), true, "fixture proof verifies");
assert.equal(
  verify(`${fixture.username}x`),
  false,
  "proof rejects another username",
);
assert.equal(
  verify(fixture.username, fixture.index.toUpperCase()),
  false,
  "uppercase index is a different store key",
);

for (let i = 0; i < 1_000; i++) {
  verify();
}

const ns = new Float64Array(iterations);
let all = true;
for (let i = 0; i < iterations; i++) {
  const start = process.hrtime.bigint();
  all &&= verify();
  ns[i] = Number(process.hrtime.bigint() - start);
}
assert.equal(all, true, "every timed call verified");

ns.sort();
const median = (ns[(iterations - 1) >> 1] + ns[iterations >> 1]) / 2;
const mean = ns.reduce((sum, x) => sum + x, 0) / iterations;

console.log(`cpu: ${cpus()[0].model}`);
console.log(`node: ${process.version}`);
console.log(`calls: ${iterations} (after 1000 warm-up calls)`);
console.log(`median: ${(median / 1000).toFixed(1)} us`);
console.log(`mean: ${(mean / 1000).toFixed(1)} us`);
