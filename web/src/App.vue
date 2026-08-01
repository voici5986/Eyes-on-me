<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { Activity, Bot, Brain, FileImage, FileText, LogOut, MonitorDot, Settings, Wifi, WifiOff } from "@lucide/vue";
import { RouterLink, RouterView } from "vue-router";
import { connectStream, fetchAuthSession, fetchDevices, loginDashboard, logoutDashboard } from "./api";
import LoginView from "./views/LoginView.vue";
import type { AuthSession, DashboardSnapshot, StreamMessage } from "./types";

const connection = ref<"connecting" | "live" | "closed">("connecting");
const deviceCount = ref(0);
const refreshToken = ref(0);
const nowMs = ref(Date.now());
const session = ref<AuthSession | null>(null);
const authBusy = ref(false);
const authError = ref<string | null>(null);

let stream: EventSource | null = null;
let tickTimer: number | null = null;

async function loadDeviceCount() {
  try {
    const response = await fetchDevices();
    deviceCount.value = response.devices.length;
  } catch {
    deviceCount.value = 0;
  }
}

async function startDashboard() {
  await loadDeviceCount();
  stream?.close();
  stream = connectStream(handleMessage);
  stream.onopen = () => { connection.value = "live"; };
  stream.onerror = () => { connection.value = "closed"; };
}

async function login(token: string) {
  authBusy.value = true;
  try {
    session.value = await loginDashboard(token);
    authError.value = null;
    await startDashboard();
  } catch {
    authError.value = "访问令牌不正确";
  } finally { authBusy.value = false; }
}

async function logout() {
  await logoutDashboard();
  stream?.close();
  stream = null;
  connection.value = "closed";
  deviceCount.value = 0;
  session.value = await fetchAuthSession();
}

function handleMessage(message: StreamMessage<DashboardSnapshot>) {
  if (message.type === "snapshot") {
    deviceCount.value = message.payload.devices.length;
    refreshToken.value += 1;
    connection.value = "live";
    return;
  }

  refreshToken.value += 1;
  connection.value = "live";
}

onMounted(async () => {
  try {
    session.value = await fetchAuthSession();
    if (session.value.authenticated) await startDashboard();
  } catch (err) {
    authError.value = err instanceof Error ? err.message : String(err);
    session.value = { authenticated: false, authRequired: true };
  }

  tickTimer = window.setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);
});

onBeforeUnmount(() => {
  stream?.close();
  if (tickTimer !== null) {
    window.clearInterval(tickTimer);
  }
});
</script>

<template>
  <div v-if="session === null" class="auth-loading"><span class="spinner" /></div>
  <LoginView v-else-if="!session.authenticated" :busy="authBusy" :error="authError" @submit="login" />
  <div v-else class="app-shell">
    <header class="app-header">
      <RouterLink class="brand" to="/" aria-label="Eyes on Me 活动概览">
        <span class="brand__mark"><Activity :size="19" :stroke-width="2.2" /></span>
        <span>
          <strong>Eyes on Me</strong>
          <small>Activity Console</small>
        </span>
      </RouterLink>

      <nav class="app-nav" aria-label="主导航">
        <RouterLink to="/"><MonitorDot :size="17" /> 活动概览</RouterLink>
        <RouterLink to="/reports"><FileText :size="17" /> 日报</RouterLink>
        <RouterLink to="/assistant"><Bot :size="17" /> 助手</RouterLink>
        <RouterLink to="/memory"><Brain :size="17" /> 记忆</RouterLink>
        <RouterLink to="/media"><FileImage :size="17" /> 采集</RouterLink>
        <RouterLink to="/settings"><Settings :size="17" /> 设置</RouterLink>
      </nav>

      <div class="app-status" :class="`is-${connection}`">
        <component :is="connection === 'live' ? Wifi : WifiOff" :size="15" />
        <span>{{ connection === "live" ? "实时连接" : connection === "connecting" ? "连接中" : "连接断开" }}</span>
        <i />
        <span>{{ deviceCount }} 台设备</span>
        <button v-if="session.authRequired" class="header-icon-button" type="button" aria-label="退出登录" title="退出登录" @click="logout"><LogOut :size="15" /></button>
      </div>
    </header>

    <main class="workspace">
      <RouterView v-slot="{ Component }">
        <KeepAlive>
          <component
            :is="Component"
            :connection="connection"
            :now-ms="nowMs"
            :refresh-token="refreshToken"
          />
        </KeepAlive>
      </RouterView>
    </main>
  </div>
</template>
