<script setup lang="ts">
import { Bot, MessageSquarePlus, Send, Sparkles, Trash2 } from "@lucide/vue";
import { nextTick, onMounted, ref } from "vue";
import { deleteAssistantConversation, fetchAssistantConversation, fetchAssistantConversations, fetchAssistantPrompts, streamAssistantReply } from "../api";
import type { AssistantConversation, AssistantMessage } from "../types";

const conversations = ref<AssistantConversation[]>([]);
const activeId = ref<string | null>(null);
const messages = ref<AssistantMessage[]>([]);
const prompts = ref<string[]>([]);
const prompt = ref("");
const useAi = ref(false);
const sending = ref(false);
const error = ref<string | null>(null);
const messageList = ref<HTMLElement | null>(null);

async function load() {
  try {
    [conversations.value, prompts.value] = await Promise.all([fetchAssistantConversations(), fetchAssistantPrompts()]);
    if (conversations.value[0]) await openConversation(conversations.value[0].id);
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
}
async function openConversation(id: string) {
  try {
    const detail = await fetchAssistantConversation(id);
    activeId.value = id;
    messages.value = detail.messages;
    await scrollToEnd();
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
}
function newConversation() { activeId.value = null; messages.value = []; prompt.value = ""; error.value = null; }
async function removeConversation(id: string) {
  if (!window.confirm("删除这段助手会话？")) return;
  try {
    await deleteAssistantConversation(id);
    conversations.value = conversations.value.filter((item) => item.id !== id);
    if (activeId.value === id) newConversation();
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
}
async function send() {
  const text = prompt.value.trim();
  if (!text || sending.value) return;
  sending.value = true;
  prompt.value = "";
  error.value = null;
  const createdAt = new Date().toISOString();
  messages.value.push({ id: `local-user-${Date.now()}`, conversationId: activeId.value ?? "new", role: "user", content: text, mode: "input", createdAt });
  const pending: AssistantMessage = { id: `local-assistant-${Date.now()}`, conversationId: activeId.value ?? "new", role: "assistant", content: "", mode: useAi.value ? "ai_enhanced" : "template", createdAt };
  messages.value.push(pending);
  await scrollToEnd();
  try {
    const reply = await streamAssistantReply(text, activeId.value, useAi.value, (token) => {
      pending.content += token;
      void scrollToEnd();
    });
    Object.assign(pending, reply.message);
    activeId.value = reply.conversation.id;
    prompts.value = reply.starterPrompts;
    conversations.value = await fetchAssistantConversations();
  } catch (err) {
    messages.value = messages.value.filter((item) => item !== pending);
    error.value = err instanceof Error ? err.message : String(err);
  } finally { sending.value = false; await scrollToEnd(); }
}
async function scrollToEnd() {
  await nextTick();
  messageList.value?.scrollTo({ top: messageList.value.scrollHeight, behavior: "smooth" });
}
function usePrompt(value: string) { prompt.value = value; void send(); }

onMounted(load);
</script>

<template>
  <div class="assistant-page">
    <aside class="assistant-sidebar">
      <header><div><span class="section-kicker">History</span><h1>工作助手</h1></div><button class="icon-button" type="button" title="新会话" aria-label="新会话" @click="newConversation"><MessageSquarePlus :size="17" /></button></header>
      <div class="assistant-conversations"><button v-for="item in conversations" :key="item.id" type="button" :class="{ 'is-active': activeId === item.id }" @click="openConversation(item.id)"><span><strong>{{ item.title }}</strong><small>{{ new Date(item.updatedAt).toLocaleString() }}</small></span><i role="button" tabindex="0" title="删除会话" aria-label="删除会话" @click.stop="removeConversation(item.id)" @keydown.enter.stop="removeConversation(item.id)"><Trash2 :size="13" /></i></button><span v-if="!conversations.length">暂无历史会话</span></div>
    </aside>

    <section class="assistant-workspace">
      <header class="assistant-header"><div><span class="assistant-avatar"><Bot :size="20" /></span><div><strong>本地工作回顾</strong><small>{{ useAi ? "AI 增强" : "基础模板" }}</small></div></div><label class="toggle-control"><input v-model="useAi" type="checkbox" /><span /><Sparkles :size="13" />AI</label></header>
      <div ref="messageList" class="assistant-messages">
        <div v-if="!messages.length" class="assistant-empty"><Bot :size="25" /><strong>从本地记录开始回顾</strong><div><button v-for="item in prompts" :key="item" type="button" @click="usePrompt(item)">{{ item }}</button></div></div>
        <article v-for="message in messages" :key="message.id" :class="`assistant-message is-${message.role}`"><span>{{ message.role === "user" ? "你" : "助手" }}</span><div>{{ message.content || "正在整理…" }}</div><small>{{ message.mode }}</small></article>
      </div>
      <p v-if="error" class="inline-error">{{ error }}</p>
      <form class="assistant-composer" @submit.prevent="send"><textarea v-model="prompt" rows="2" maxlength="8000" placeholder="询问今天、昨天、某个日期或最近一周的活动" aria-label="助手问题" @keydown.meta.enter.prevent="send" @keydown.ctrl.enter.prevent="send" /><button class="primary-button" type="submit" :disabled="sending || !prompt.trim()" title="发送" aria-label="发送"><Send :size="16" /></button></form>
    </section>
  </div>
</template>
