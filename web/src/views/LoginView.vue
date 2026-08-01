<script setup lang="ts">
import { Activity, KeyRound, LogIn } from "@lucide/vue";
import { nextTick, onMounted, ref } from "vue";

defineProps<{ busy: boolean; error: string | null }>();
const emit = defineEmits<{ submit: [token: string] }>();
const token = ref("");
const input = ref<HTMLInputElement | null>(null);

function submit() {
  const value = token.value.trim();
  if (value) emit("submit", value);
}

onMounted(() => void nextTick(() => input.value?.focus()));
</script>

<template>
  <main class="login-view">
    <section class="login-panel">
      <span class="login-panel__mark"><Activity :size="23" /></span>
      <div>
        <span class="section-kicker">Eyes on Me</span>
        <h1>访问活动控制台</h1>
        <p>输入 Dashboard 访问令牌。</p>
      </div>
      <form @submit.prevent="submit">
        <label for="dashboard-token">访问令牌</label>
        <div class="login-input"><KeyRound :size="17" /><input id="dashboard-token" ref="input" v-model="token" type="password" autocomplete="current-password" /></div>
        <p v-if="error" class="inline-error">{{ error }}</p>
        <button class="primary-button" type="submit" :disabled="busy || !token.trim()"><LogIn :size="16" />{{ busy ? "正在验证" : "进入控制台" }}</button>
      </form>
    </section>
  </main>
</template>
