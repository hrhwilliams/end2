import { defineStore } from "pinia";
import type { DeviceId, DeviceInfo } from "../types/device";
import type { UserId } from "../types/user";
import { ref } from "vue";
import { Err, Ok, request, type ApiResult } from "../api";
import { useUserStore } from "./user";
import {
  fetchDeviceKeys,
  readsValidators,
  verifiedIndex,
} from "../services/directory";

export const useDeviceStore = defineStore("device", () => {
  const devices = ref<Record<UserId, Record<DeviceId, DeviceInfo>>>({});

  async function fetchOurDevices(): Promise<ApiResult<DeviceInfo[]>> {
    const userStore = useUserStore();
    const me = userStore.me;

    if (!me) {
      return Err({ status: 0, message: "not logged in" });
    }

    const records = devices.value[me.id];
    if (records) {
      return Ok(Object.values(records));
    }

    const response = await request<DeviceInfo[]>("/me/devices", "GET");
    if (!response.ok) {
      return Err(response.error);
    }

    devices.value[me.id] = Object.fromEntries(
      response.value.map((d) => [d.device_id, d]),
    );
    return Ok(response.value);
  }

  async function fetchFromValidators(
    userId: string,
    deviceId: string,
  ): Promise<ApiResult<DeviceInfo>> {
    const user = await useUserStore().getUser(userId);
    if (!user) {
      return Err({ status: 0, message: `unknown user ${userId}` });
    }
    const index = verifiedIndex(user);
    if (!index.ok) {
      return index;
    }
    const keys = await fetchDeviceKeys(index.value, deviceId);
    if (!keys.ok) {
      return keys;
    }
    return Ok({ device_id: deviceId, user_id: userId, ...keys.value });
  }

  async function fetchDevice(
    userId: string,
    deviceId: string,
  ): Promise<ApiResult<DeviceInfo>> {
    if (devices.value[userId]?.[deviceId]) {
      return Ok(devices.value[userId][deviceId]);
    }

    const response = readsValidators
      ? await fetchFromValidators(userId, deviceId)
      : await request<DeviceInfo>(`/user/${userId}/device/${deviceId}`, "GET");
    if (response.ok) {
      if (!devices.value[userId]) {
        devices.value[userId] = {};
      }
      devices.value[userId][deviceId] = response.value;
    }

    return response;
  }

  return {
    devices,
    fetchOurDevices,
    fetchDevice,
  };
});
