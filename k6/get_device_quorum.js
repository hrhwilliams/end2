import http from 'k6/http';
import { check } from 'k6';
import { Counter, Trend } from 'k6/metrics';
import encoding from 'k6/encoding';

// The secure read path: keys come straight from the validators, never from the backend.
// Mirrors quorumQuery in crates/end2-wasm-client/frontend/src/services/directory.ts.
const BASE = __ENV.BASE || 'https://chat.fiatlux.dev';
const VALIDATORS = [0, 1, 2, 3].map((n) => `${BASE}/comet/${n}`);
const NEEDED = Math.floor((VALIDATORS.length - 1) / 3) + 1;
// The client asks every validator. ASK=2 asks only f+1 first, falling back to the rest on a split,
// which is faster but cannot notice two colluding validators that happen to be the pair asked.
const ASK = Number(__ENV.ASK || VALIDATORS.length);

const quorumDuration = new Trend('get_device_quorum_ms', true);
const fallbacks = new Counter('quorum_fallbacks');

const DEVICE_KEYS = {
  ed25519: "PbCMOYxMo6vTYC9uGiGcqHpJIIpjWOtvWG2sVSX+TKg",
  x25519:  "WS2NBb75L0Gcw96kNGfJgh56aPrYxRgjJG3hOifEQ04",
  signature: "NPA6Z/4omyLxqz3SACx/NDqifqutUxyCwcL0KBAdCUm4SfZ7vvZ1p2nU6o5VTC9afUUqFhTLPEXZfx8GllLGAA",
};

function hex(s) {
  let out = '';
  for (let i = 0; i < s.length; i++) out += s.charCodeAt(i).toString(16).padStart(2, '0');
  return out;
}

// The base64 value, null when the key is absent, undefined when the validator gave no answer.
function answerOf(res) {
  if (res.status !== 200) return undefined;
  try {
    const r = JSON.parse(res.body).result.response;
    return r.code === 0 ? (r.value || '') : null;
  } catch (_) {
    return undefined;
  }
}

function agreed(answers) {
  const counts = {};
  for (const a of answers) {
    if (a !== undefined) counts[a] = (counts[a] || 0) + 1;
  }
  const values = Object.keys(counts).filter((v) => counts[v] >= NEEDED);
  return values.length === 1 ? values[0] : undefined;
}

function ask(urls, path, data) {
  const query = `path=${encodeURIComponent(`"${path}"`)}&data=0x${hex(data)}`;
  return http.batch(urls.map((u) => ['GET', `${u}/abci_query?${query}`])).map(answerOf);
}

function quorumQuery(path, data) {
  const start = Math.floor(Math.random() * VALIDATORS.length);
  const order = VALIDATORS.map((_, i) => VALIDATORS[(start + i) % VALIDATORS.length]);
  const answers = ask(order.slice(0, ASK), path, data);
  let value = agreed(answers);
  if (value === undefined && ASK < VALIDATORS.length) {
    fallbacks.add(1);
    value = agreed(answers.concat(ask(order.slice(ASK), path, data)));
  }
  return value;
}

export default function () {
  const username = `vu_${Math.random().toString(36)}`;
  const password = '1234';

  http.post(`${BASE}/api/auth/register`,
    JSON.stringify({ username, password, confirm_password: password }),
    { headers: { 'Content-Type': 'application/json' } }
  );

  const newDevice = http.post(`${BASE}/api/me/device`);
  let device_id;
  try {
    ({ device_id } = JSON.parse(newDevice.body));
  } catch (_) {
    console.error(`VU ${__VU} iter ${__ITER}: failed to create device: ${newDevice.body}`);
    return;
  }

  const put = http.put(`${BASE}/api/me/device`,
    JSON.stringify({ device_id, ...DEVICE_KEYS }),
    { headers: { 'Content-Type': 'application/json' } }
  );
  if (put.status !== 200) {
    console.error(`VU ${__VU} iter ${__ITER}: PUT failed: ${put.body}`);
    return;
  }

  // The index comes with its VRF proof; verifying it is a one-off per contact, measured separately.
  const index = JSON.parse(http.get(`${BASE}/api/me`).body).directory.index;

  const start = Date.now();
  const value = quorumQuery('device', `${index}:${device_id}`);
  quorumDuration.add(Date.now() - start);

  const keys = value ? JSON.parse(encoding.b64decode(value, 'std', 's')) : null;
  check(keys, {
    'quorum agreed on keys': (k) => k !== null,
    'has x25519':            (k) => !!k && k.x25519.length > 0,
    'has ed25519':           (k) => !!k && k.ed25519.length > 0,
  });
  if (!keys) {
    console.error(`VU ${__VU} iter ${__ITER}: no quorum for ${index}:${device_id}`);
  }
}
