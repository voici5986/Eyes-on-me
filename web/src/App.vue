<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { RouterLink, RouterView } from "vue-router";
import { connectStream, fetchDevices } from "./api";
import type { DashboardSnapshot, StreamMessage } from "./types";

const connection = ref<"connecting" | "live" | "closed">("connecting");
const deviceCount = ref(0);
const refreshToken = ref(0);
const nowMs = ref(Date.now());

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
  await loadDeviceCount();

  stream = connectStream(handleMessage);
  stream.onopen = () => {
    connection.value = "live";
  };
  stream.onerror = () => {
    connection.value = "closed";
  };

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
  <main class="shell">
    <section class="hero">
      <div>
        <p class="eyebrow">Eyes on Me / Rust Monolith</p>
        <h1>Eyes on Me</h1>
        <p class="lede">
          打开首页就是全局分析，再继续钻进单机明细和设备分析。现在这套页面就是 Eyes on Me 的监控工作台。
        </p>
      </div>

      <div class="status-card">
        <span class="label">Stream</span>
        <strong>{{ connection }}</strong>
        <span class="muted">Devices online: {{ deviceCount }}</span>
      </div>
    </section>

    <nav class="top-nav">
      <!-- <RouterLink class="nav-link" to="/">设备汇总</RouterLink> -->
      <!-- <RouterLink class="nav-link" to="/analysis">分析页</RouterLink> -->
      <!-- <span class="muted">选择设备后进入明细页或设备分析页</span> -->
    </nav>

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
</template>
