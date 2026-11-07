import { Device } from "../../../pkg/end2_wasm_client";
import type {
  DeviceInfo,
  DeviceOneTimeKeys,
  Otk,
  UploadDeviceKeys,
  UploadOtks,
} from "../types/device";
import type {
  DecryptedMessage,
  EncryptedMessagePayload,
  InboundChatMessage,
} from "../types/message";
import type { UserInfo } from "../types/user";
import { markRaw } from "vue";
import { Err, Ok, request, type ApiResult } from "../api";
import { db, type SessionPickle } from "../db";
import { fetchIndexDevices, readsValidators, verifiedIndex } from "./directory";
import { createWalletClient, http } from 'viem'
import type { WalletClient } from 'viem'
import { generatePrivateKey, privateKeyToAccount } from 'viem/accounts'
import type { PrivateKeyAccount } from 'viem/accounts'
import { foundry } from 'viem/chains'

type EncryptionOutput = {
  session: SessionPickle;
  payload: EncryptedMessagePayload;
};

type DecryptionOutput = {
  session: SessionPickle;
  payload: DecryptedMessage;
};

let context: Device | null = null;
let wallet_client: WalletClient | null = null;

export function getDeviceId(): string | null {
  if (context) {
    return context.device_id();
  }
  return null;
}

export async function initDevice(user: UserInfo): Promise<ApiResult<null>> {
  const device_pickle = await db.account.get(user.id);

  if (device_pickle) {
    context = markRaw(Device.try_from_pickle(device_pickle));
  } else {
    let response = await request<DeviceInfo>("/me/device", "POST");
    if (!response.ok) {
      return Err(response.error);
    }

    const device = Device.new(response.value.device_id);
    context = markRaw(device);

    const keys = device.keys() as UploadDeviceKeys;
    response = await request<DeviceInfo>("/me/device", "PUT", keys);
    if (readsValidators) {
      const confirmed = await confirmRegistration(
        user,
        device.device_id(),
        keys,
        response,
      );
      if (!confirmed.ok) {
        return confirmed;
      }
    } else if (!response.ok) {
      return Err(response.error);
    }

    await saveDevice(user.id);
  }

  const wallet_pickle = await db.wallet.get(user.id);
  let account: PrivateKeyAccount;

  if (wallet_pickle) {
    account = privateKeyToAccount(wallet_pickle.private_key);
  } else {
    const eth_key = generatePrivateKey();
    account = privateKeyToAccount(eth_key);

    await saveWallet(user.id, eth_key);
  }

  wallet_client = markRaw(createWalletClient({ 
    account, 
    chain: foundry,
    transport: http()
  }));

  return Ok(null);
}

const IMPERSONATION_WARNING =
  "Your directory index already has a root key. If you have not registered another device, the provider may be impersonating you.";

// Reads the validators, not the provider's reply, to confirm this device's keys are under the user's own index.
async function confirmRegistration(
  user: UserInfo,
  deviceId: string,
  keys: UploadDeviceKeys,
  put: ApiResult<DeviceInfo>,
): Promise<ApiResult<null>> {
  const index = verifiedIndex(user);
  if (!index.ok) {
    return index;
  }
  const devices = await fetchIndexDevices(index.value);
  if (!devices.ok) {
    return devices;
  }

  const mine = devices.value[deviceId];
  if (mine?.x25519 === keys.x25519 && mine.ed25519 === keys.ed25519) {
    return Ok(null);
  }
  if (Object.keys(devices.value).length > 0) {
    return Err({ status: 0, message: IMPERSONATION_WARNING });
  }
  return put.ok
    ? Err({
        status: 0,
        message: "this device's keys are not in the key directory",
      })
    : Err(put.error);
}

export async function saveDevice(user_id: string): Promise<void> {
  try {
    if (context) {
      await db.account.upsert(user_id, context.to_pickle());
    } else {
      console.warn("attempted to save without device context initialized");
    }
  } catch (e) {
    console.error("failed to save device context: ", e);
  }
}

export async function saveWallet(user_id: string, private_key: `0x${string}`): Promise<void> {
  try {
    await db.wallet.upsert(user_id, { private_key });
  } catch (e) {
    console.error("failed to save wallet: ", e);
  }
}

export async function signMessage(message: string): Promise<`0x${string}`> {
  if (!wallet_client || !wallet_client.account) {
    throw new Error("wallet not initialized");
  }
  return wallet_client.signMessage({ account: wallet_client.account, message });
}

export async function ensureOtks(userId: string): Promise<ApiResult<unknown>> {
  if (!context) {
    return Err({ status: 0, message: "no context" });
  }

  const deviceId = getDeviceId();
  const response = await request<DeviceOneTimeKeys>(
    `/me/device/${deviceId}/otks`,
    "GET",
  );

  if (!response.ok) {
    return Err(response.error);
  }

  if (response.value.otks.length < 10) {
    const newOtks = context.gen_otks(50) as UploadOtks;
    const response2 = await request<void>(
      `/me/device/${deviceId}/otks`,
      "POST",
      newOtks,
    );

    if (response2.ok) {
      await saveDevice(userId);
    }

    return response2;
  }

  return response;
}

export async function encryptMessage(
  channelId: string,
  device: DeviceInfo,
  plaintext: string,
  userId: string,
): Promise<ApiResult<EncryptedMessagePayload>> {
  if (!context) {
    return Err({ status: 0, message: "no context" });
  }

  if (device.device_id === context.device_id()) {
    return Err({ status: 0, message: "skipping encryption for self" });
  }

  let session = await db.sessions.get(channelId + ":" + device.device_id);
  let output: EncryptionOutput;

  if (session) {
    output = context.encrypt(session, device, plaintext);
  } else {
    const response = await request<Otk>(
      `/user/${device.user_id}/device/${device.device_id}/otk`,
      "POST",
    );
    if (!response.ok) {
      return response;
    }

    output = context.encrypt_otk(device, plaintext, response.value.otk);
    await saveDevice(userId);
  }

  await db.sessions.upsert(channelId + ":" + device.device_id, output.session);
  return Ok(output.payload);
}

export async function decryptMessage(
  device: DeviceInfo,
  message: InboundChatMessage,
  userId: string,
): Promise<ApiResult<DecryptedMessage>> {
  if (!context) {
    return Err({ status: 0, message: "no context" });
  }

  let output: DecryptionOutput;

  if (message.is_pre_key) {
    const session = await db.sessions.get(
      message.channel_id + ":" + message.device_id,
    );
    if (session) {
      output = context.decrypt(session, device, message);
    } else {
      output = context.decrypt_otk(device, message);
      await saveDevice(userId);
    }
  } else {
    const session = await db.sessions.get(
      message.channel_id + ":" + message.device_id,
    );
    if (session) {
      output = context.decrypt(session, device, message);
    } else {
      return Err({ status: 0, message: "missing session for message" });
    }
  }

  await db.sessions.upsert(
    message.channel_id + ":" + message.device_id,
    output.session,
  );
  return Ok(output.payload);
}
