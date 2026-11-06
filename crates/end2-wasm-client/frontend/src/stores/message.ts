import { defineStore } from "pinia";
import { v7 } from "uuid";
import Dexie from "dexie";
import { db } from "../db";
import { Err, Ok, request, type ApiResult } from "../api";
import { useUserStore } from "./user";
import { useDeviceStore } from "./device";
import { useChannelStore } from "./channel";
import {
  getDeviceId,
  encryptMessage,
  decryptMessage,
} from "../services/crypto";
import type {
  DecryptedMessage,
  EncryptedMessage,
  InboundChatMessage,
  MessageReceivedEvent,
} from "../types/message";

type PendingMessage = {
  channel_id: string;
  author_id: string;
  plaintext: string;
};

export const useMessageStore = defineStore("message", () => {
  const userStore = useUserStore();
  const pendingMessages = new Map<string, PendingMessage>();

  async function persistMessage(msg: DecryptedMessage): Promise<void> {
    const timestamp =
      msg.timestamp instanceof Date ? msg.timestamp : new Date(msg.timestamp);

    await db.messages.put({ ...msg, timestamp });
  }

  async function handleInbound(message: InboundChatMessage): Promise<void> {
    const deviceStore = useDeviceStore();

    const device = await deviceStore.fetchDevice(
      message.author_id,
      message.device_id,
    );
    if (!device.ok) {
      console.error(
        "failed to fetch device for message:",
        message.message_id,
        device.error,
      );
      return;
    }

    if (!userStore.me) {
      console.error("failed to decrypt message: not logged in");
      return;
    }

    const decrypted = await decryptMessage(
      device.value,
      message,
      userStore.me.id,
    );
    if (!decrypted.ok) {
      console.error(
        "failed to decrypt message:",
        message.message_id,
        decrypted.error,
      );
      return;
    }

    await persistMessage(decrypted.value);
  }

  async function sendMessage(
    channelId: string,
    plaintext: string,
  ): Promise<ApiResult<void>> {
    const channelStore = useChannelStore();

    if (!userStore.me) {
      return Err({ status: 0, message: "not logged in" });
    }

    const deviceId = getDeviceId();
    if (!deviceId) {
      return Err({ status: 0, message: "no device context" });
    }

    const devices = channelStore.getDevices(channelId);
    const message: EncryptedMessage = {
      message_id: `msg_${v7()}`,
      device_id: deviceId,
      channel_id: channelId,
      payloads: [],
    };

    const deviceStore = useDeviceStore();
    for (const listed of devices) {
      if (listed.device_id !== deviceId) {
        // The provider's channel info names the devices. Their keys must come from fetchDevice, which checks the directory.
        const device = await deviceStore.fetchDevice(
          listed.user_id,
          listed.device_id,
        );
        if (!device.ok) {
          console.error(
            "failed to fetch device for encryption:",
            listed.device_id,
            device.error,
          );
          continue;
        }
        const payload = await encryptMessage(
          channelId,
          device.value,
          plaintext,
          userStore.me.id,
        );
        if (payload.ok) {
          message.payloads.push(payload.value);
        } else {
          console.error(
            "failed to encrypt for device:",
            listed.device_id,
            payload.error,
          );
        }
      }
    }

    pendingMessages.set(message.message_id, {
      channel_id: channelId,
      author_id: userStore.me.id,
      plaintext,
    });

    const response = await request(
      `/channel/${channelId}/msg`,
      "POST",
      message,
    );
    if (!response.ok) {
      pendingMessages.delete(message.message_id);
      return Err(response.error);
    }

    return Ok(undefined as void);
  }

  async function fetchHistory(channelId: string): Promise<void> {
    const deviceStore = useDeviceStore();

    const deviceId = getDeviceId();
    if (!deviceId) {
      return;
    }

    if (!userStore.me) {
      return;
    }

    const lastMessage = await db.messages
      .where("[channel_id+timestamp]")
      .between([channelId, Dexie.minKey], [channelId, Dexie.maxKey])
      .last();

    const params = new URLSearchParams({ device: deviceId });
    if (lastMessage) {
      params.append("after", lastMessage.message_id);
    }

    const response = await request<InboundChatMessage[]>(
      `/channel/${channelId}/history?${params}`,
      "GET",
    );
    if (!response.ok) {
      console.error("failed to fetch chat history:", response.error);
      return;
    }

    for (const msg of response.value) {
      const device = await deviceStore.fetchDevice(
        msg.author_id,
        msg.device_id,
      );
      if (!device.ok) {
        console.error(
          "failed to fetch device for history message:",
          msg.message_id,
          device.error,
        );
        continue;
      }

      const decrypted = await decryptMessage(
        device.value,
        msg,
        userStore.me.id,
      );
      if (decrypted.ok) {
        await persistMessage(decrypted.value);
      } else {
        console.error(
          "failed to decrypt history message:",
          msg.message_id,
          decrypted.error,
        );
      }
    }
  }

  async function confirmMessage(event: MessageReceivedEvent): Promise<void> {
    const pending = pendingMessages.get(event.message_id);
    if (!pending) {
      console.warn(
        "received confirmation for unknown message:",
        event.message_id,
      );
      return;
    }

    pendingMessages.delete(event.message_id);

    await persistMessage({
      message_id: event.message_id,
      channel_id: pending.channel_id,
      author_id: pending.author_id,
      plaintext: pending.plaintext,
      timestamp: new Date(event.timestamp),
    });
  }

  return {
    handleInbound,
    sendMessage,
    confirmMessage,
    fetchHistory,
  };
});
