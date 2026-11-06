<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useQuery } from "@tanstack/vue-query";
import { useRoute } from "vue-router";
import { useUserStore } from "./stores/user";
import { request, type ApiError } from "./api";
import type { UserInfo } from "./types/user";
import { useWebSocketStore } from "./stores/socket";
import { initDevice, ensureOtks, getDeviceId } from "./services/crypto";
import ErrorMessage from "./components/ErrorMessage.vue";

const route = useRoute();
let pageTitle = computed(() => route.meta.title || "End2");

const userStore = useUserStore();
const socket = useWebSocketStore();

const error = ref<ApiError | null>(null);

useQuery({
  queryKey: ["me"],
  queryFn: async () => {
    const response = await request<UserInfo>("/me", "GET");

    if (response.ok) {
      userStore.login(response.value);
      error.value = null;
      return response.value;
    } else {
      if (response.error.status != 401) {
        error.value = response.error;
      }
      return null;
    }
  },
  retry: false,
});

watch(
  () => userStore.me,
  async (me) => {
    if (me) {
      const response = await initDevice(me);
      if (!response.ok) {
        error.value = response.error;
        return;
      }

      const response2 = await ensureOtks(me.id);
      if (!response2.ok) {
        error.value = response2.error;
        return;
      }

      const deviceId = getDeviceId();
      if (deviceId) {
        socket.connect(deviceId);
      }
    } else {
      socket.disconnect();
    }
  },
);
</script>

<template>
  <div class="navbar">
    <header>
      <h1>{{ pageTitle }}</h1>
      <nav>
        <div>
          <p v-if="userStore.logged_in && userStore.me">
            Logged in as
            <span v-if="userStore.me.nickname"
              ><strong>{{ userStore.me.nickname }}</strong> ({{
                userStore.me.username
              }})</span
            >
            <strong v-else>{{ userStore.me!.username }}</strong>
            | <RouterLink to="/">Home</RouterLink> |
            <RouterLink to="/chats">Chats</RouterLink> |
            <RouterLink to="/settings">Settings</RouterLink> |
            <RouterLink to="/logout">Log Out</RouterLink>
          </p>
          <p v-else>
            <RouterLink to="/">Home</RouterLink>
            | <RouterLink to="/login">Log In</RouterLink> |
            <a href="/api/auth/discord"
              >Log in with Discord</a
            >
            | <RouterLink to="/register">Register</RouterLink>
          </p>
        </div>
      </nav>
    </header>
    <hr />
  </div>
  <main>
    <router-view></router-view>
    <ErrorMessage
      v-if="error"
      :status="error.status"
      :message="error.message"
      :detail="error.detail"
    >
    </ErrorMessage>
  </main>
</template>
