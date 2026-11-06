import { verify_directory_index } from "../../../pkg/end2_wasm_client";
import { Err, Ok, type ApiResult } from "../api";
import type { UserInfo } from "../types/user";

const VRF_PUBLIC_KEY = import.meta.env.VITE_VRF_PUBLIC_KEY ?? "";
const VALIDATORS = (import.meta.env.VITE_COMET_RPC_URLS ?? "")
  .split(",")
  .map((url) => url.trim().replace(/\/+$/, ""))
  .filter((url) => url.length > 0);

// Unset means the provider serves keys, so the Postgres and Ethereum controls still run.
export const readsValidators = VALIDATORS.length > 0;

export type DeviceKeys = { x25519: string; ed25519: string };
type RawKeys = { x25519: number[]; ed25519: number[] };

const fail = (message: string): ApiResult<never> => Err({ status: 0, message });

export function verifiedIndex(user: UserInfo): ApiResult<string> {
  const d = user.directory;
  if (
    !d ||
    !verify_directory_index(VRF_PUBLIC_KEY, user.username, d.index, d.proof)
  ) {
    return fail(`directory proof for ${user.username} did not verify`);
  }
  return Ok(d.index);
}

function hexOfUtf8(s: string): string {
  return Array.from(new TextEncoder().encode(s), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}

function toBase64(bytes: number[]): string {
  return btoa(String.fromCharCode(...bytes)).replace(/=+$/, "");
}

function toKeys(raw: RawKeys): DeviceKeys {
  return { x25519: toBase64(raw.x25519), ed25519: toBase64(raw.ed25519) };
}

// The base64 value, null when the key is absent, undefined when the validator gave no answer.
async function ask(
  url: string,
  path: string,
  data: string,
): Promise<string | null | undefined> {
  try {
    const query = `path=${encodeURIComponent(`"${path}"`)}&data=0x${hexOfUtf8(data)}`;
    const response = await fetch(`${url}/abci_query?${query}`);
    if (!response.ok) {
      return undefined;
    }
    const result = (await response.json())?.result?.response;
    if (!result) {
      return undefined;
    }
    return result.code === 0 ? (result.value ?? "") : null;
  } catch {
    return undefined;
  }
}

// f+1 identical answers, f = floor((n-1)/3), means at least one honest validator vouches for the value.
async function quorumQuery(
  path: string,
  data: string,
): Promise<ApiResult<string | null>> {
  const answers = await Promise.all(
    VALIDATORS.map((url) => ask(url, path, data)),
  );
  const needed = Math.floor((VALIDATORS.length - 1) / 3) + 1;

  const counts = new Map<string | null, number>();
  for (const answer of answers) {
    if (answer !== undefined) {
      counts.set(answer, (counts.get(answer) ?? 0) + 1);
    }
  }
  const agreed = [...counts].filter(([, n]) => n >= needed);

  if (agreed.length === 1) {
    return Ok(agreed[0]![0]);
  }
  if (agreed.length > 1) {
    return fail(`validators disagree on ${path} ${data}`);
  }
  const answered = answers.filter((a) => a !== undefined).length;
  return fail(
    `no validator quorum for ${path} ${data}: ${needed} matching answers needed, ${answered} of ${VALIDATORS.length} validators answered`,
  );
}

export async function fetchDeviceKeys(
  index: string,
  deviceId: string,
): Promise<ApiResult<DeviceKeys>> {
  const answer = await quorumQuery("device", `${index}:${deviceId}`);
  if (!answer.ok) {
    return answer;
  }
  if (answer.value === null) {
    return fail(`device ${deviceId} is not in the key directory`);
  }
  return Ok(toKeys(JSON.parse(atob(answer.value))));
}

export async function fetchIndexDevices(
  index: string,
): Promise<ApiResult<Record<string, DeviceKeys>>> {
  const answer = await quorumQuery("devices", index);
  if (!answer.ok) {
    return answer;
  }
  if (answer.value === null) {
    return Ok({});
  }
  const raw: Record<string, RawKeys> = JSON.parse(atob(answer.value));
  return Ok(
    Object.fromEntries(
      Object.entries(raw).map(([id, keys]) => [id, toKeys(keys)]),
    ),
  );
}
