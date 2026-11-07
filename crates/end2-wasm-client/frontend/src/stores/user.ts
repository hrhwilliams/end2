import { defineStore } from "pinia";
import type { UserId, UserInfo } from "../types/user";
import { Err, request, type ApiResult } from "../api";
import { computed, ref } from "vue";

export const useUserStore = defineStore("user", () => {
  const current_user_id = ref<UserId | null>(null);
  const users = ref<Record<UserId, UserInfo>>({});

  const me = computed(() => {
    if (current_user_id.value && users.value[current_user_id.value]) {
      return users.value[current_user_id.value];
    }
    return null;
  });

  const logged_in = computed(() => {
    return (
      current_user_id.value !== null &&
      users.value[current_user_id.value] !== null
    );
  });

  function login(user: UserInfo) {
    current_user_id.value = user.id;
    users.value[user.id] = user;
  }

  function logout() {
    current_user_id.value = null;
    users.value = {};
  }

  async function fetch_user_info(user_id: string) {
    if (users.value[user_id]?.directory) {
      return;
    }

    let response;
    if (user_id === current_user_id.value) {
      response = await request<UserInfo>("/me", "GET");
    } else {
      response = await request<UserInfo>(`/user/${user_id}`, "GET");
    }

    if (response.ok) {
      users.value[user_id] = response.value;
    } else {
      console.error("failed to fetch user_info");
    }
  }

  async function getUser(user_id: string): Promise<UserInfo | null> {
    await fetch_user_info(user_id);
    return users.value[user_id] ?? null;
  }

  function updateUser(user: UserInfo) {
    users.value[user.id] = { ...users.value[user.id], ...user };
  }

  function has_nickname(user_id: string) {
    const user = users.value[user_id];

    if (user) {
      return user.nickname !== null;
    } else {
      fetch_user_info(user_id);
      return false;
    }
  }

  function get_username(user_id: string) {
    const user = users.value[user_id];

    if (user) {
      return user.username;
    } else {
      fetch_user_info(user_id);
      return "...";
    }
  }

  function get_display_name(user_id: string) {
    const user = users.value[user_id];

    if (user) {
      return user.nickname || user.username;
    } else {
      fetch_user_info(user_id);
      return "...";
    }
  }

  async function change_nickname(nickname: string): Promise<ApiResult<void>> {
    if (!current_user_id.value) {
      return Err({
        status: 0,
        message: "not logged in",
      });
    }

    const response = await request<void>("/me/nickname", "POST", {
      nickname: nickname.trim(),
    });

    if (response.ok) {
      const record = users.value[current_user_id.value];
      if (record) {
        record.nickname = nickname.trim();
      }
    }

    return response;
  }
  return {
    me,
    logged_in,
    login,
    logout,
    has_nickname,
    get_username,
    get_display_name,
    change_nickname,
    getUser,
    updateUser,
  };
});
