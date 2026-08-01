import type {
  AnalysisRange,
  AnalysisOverviewResponse,
  ActivitySearchResponse,
  AuthSession,
  DashboardSnapshot,
  DeviceAnalysisResponse,
  DeviceDetailResponse,
  DevicesResponse,
  StreamMessage,
  DailyReport,
  MemoryReindexResponse,
  MemorySearchResponse,
  ScreenshotListResponse,
  MediaStatus,
  MediaCleanupResponse,
  AgentDiagnosticsResponse,
  ScreenshotRecord,
  TimelineFilters,
  TimelineResponse,
  DeletionSummary,
  DeviceRecordingState,
  ReviewSettings,
  WorkSessionResponse,
  AssistantConversation,
  AssistantConversationDetail,
  AssistantReply,
  RemoteMirrorStatus
} from "./types";
import { DEFAULT_ANALYSIS_RANGE } from "./lib/analysis-range";

const responseCache = new Map<string, unknown>();
const inflightCache = new Map<string, Promise<unknown>>();

class ApiError extends Error {
  status: number;

  constructor(message: string, status: number) {
    super(message);
    this.status = status;
  }
}

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...init,
    headers: {
      accept: "application/json",
      ...init?.headers
    }
  });

  if (!response.ok) {
    throw new ApiError(`Failed to fetch ${url}: HTTP ${response.status}`, response.status);
  }

  return response.json() as Promise<T>;
}

export async function fetchAuthSession(): Promise<AuthSession> {
  return fetchJson<AuthSession>("/api/auth/session");
}

export async function loginDashboard(token: string): Promise<AuthSession> {
  const session = await fetchJson<AuthSession>("/api/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ token })
  });
  responseCache.clear();
  inflightCache.clear();
  return session;
}

export async function logoutDashboard(): Promise<AuthSession> {
  const session = await fetchJson<AuthSession>("/api/auth/logout", { method: "POST" });
  responseCache.clear();
  inflightCache.clear();
  return session;
}

async function fetchJsonCached<T>(url: string, force = false): Promise<T> {
  if (force) {
    responseCache.delete(url);
    inflightCache.delete(url);
  }

  if (responseCache.has(url)) {
    return responseCache.get(url) as T;
  }

  const inflight = inflightCache.get(url);
  if (inflight) {
    return inflight as Promise<T>;
  }

  const request = fetchJson<T>(url)
    .then((payload) => {
      responseCache.set(url, payload);
      inflightCache.delete(url);
      return payload;
    })
    .catch((error) => {
      inflightCache.delete(url);
      throw error;
    });

  inflightCache.set(url, request as Promise<unknown>);
  return request;
}

function readCached<T>(url: string): T | null {
  return (responseCache.get(url) as T | undefined) ?? null;
}

function devicesUrl(): string {
  return "/api/devices";
}

function analysisOverviewUrl(range: AnalysisRange = DEFAULT_ANALYSIS_RANGE): string {
  return `/api/analysis?range=${encodeURIComponent(range)}`;
}

function deviceDetailUrl(deviceId: string): string {
  return `/api/devices/${encodeURIComponent(deviceId)}`;
}

function deviceAnalysisUrl(deviceId: string, range: AnalysisRange = DEFAULT_ANALYSIS_RANGE): string {
  return `/api/devices/${encodeURIComponent(deviceId)}/analysis?range=${encodeURIComponent(range)}`;
}

export async function fetchCurrent(force = false): Promise<DashboardSnapshot> {
  return fetchJsonCached<DashboardSnapshot>("/api/current", force);
}

export function peekDevices(): DevicesResponse | null {
  return readCached<DevicesResponse>(devicesUrl());
}

export async function fetchDevices(force = false): Promise<DevicesResponse> {
  return fetchJsonCached<DevicesResponse>(devicesUrl(), force);
}

export function peekDeviceDetail(deviceId: string): DeviceDetailResponse | null {
  return readCached<DeviceDetailResponse | null>(deviceDetailUrl(deviceId));
}

export async function fetchDeviceDetail(deviceId: string, force = false): Promise<DeviceDetailResponse | null> {
  const url = deviceDetailUrl(deviceId);

  try {
    return await fetchJsonCached<DeviceDetailResponse>(url, force);
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      responseCache.set(url, null);
      return null;
    }

    throw error;
  }
}

export function peekAnalysisOverview(range: AnalysisRange = DEFAULT_ANALYSIS_RANGE): AnalysisOverviewResponse | null {
  return readCached<AnalysisOverviewResponse>(analysisOverviewUrl(range));
}

export async function fetchAnalysisOverview(
  range: AnalysisRange = DEFAULT_ANALYSIS_RANGE,
  force = false
): Promise<AnalysisOverviewResponse> {
  return fetchJsonCached<AnalysisOverviewResponse>(analysisOverviewUrl(range), force);
}

export function peekDeviceAnalysis(
  deviceId: string,
  range: AnalysisRange = DEFAULT_ANALYSIS_RANGE
): DeviceAnalysisResponse | null {
  return readCached<DeviceAnalysisResponse | null>(deviceAnalysisUrl(deviceId, range));
}

export async function fetchDeviceAnalysis(
  deviceId: string,
  range: AnalysisRange = DEFAULT_ANALYSIS_RANGE,
  force = false
): Promise<DeviceAnalysisResponse | null> {
  const url = deviceAnalysisUrl(deviceId, range);

  try {
    return await fetchJsonCached<DeviceAnalysisResponse>(url, force);
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) {
      responseCache.set(url, null);
      return null;
    }

    throw error;
  }
}

export function connectStream(onMessage: (message: StreamMessage<DashboardSnapshot>) => void): EventSource {
  const stream = new EventSource("/api/stream");

  stream.addEventListener("message", (event) => {
    const parsed = JSON.parse(event.data) as StreamMessage<DashboardSnapshot>;
    onMessage(parsed);
  });

  return stream;
}

export async function searchActivities(query: string, deviceId?: string, limit = 50): Promise<ActivitySearchResponse> {
  const params = new URLSearchParams({ q: query, limit: String(limit) });
  if (deviceId) params.set("deviceId", deviceId);
  return fetchJson<ActivitySearchResponse>(`/api/search/activities?${params.toString()}`);
}

export async function fetchDeviceScreenshots(deviceId: string, limit = 100): Promise<ScreenshotListResponse> {
  return fetchJson<ScreenshotListResponse>(`/api/devices/${encodeURIComponent(deviceId)}/screenshots?limit=${limit}`);
}

export async function fetchScreenshots(limit = 100): Promise<ScreenshotListResponse> {
  return fetchJson<ScreenshotListResponse>(`/api/screenshots?limit=${limit}`);
}

export async function fetchMediaStatus(): Promise<MediaStatus> {
  return fetchJson<MediaStatus>("/api/media/status");
}

export async function fetchRemoteMirrorStatus(): Promise<RemoteMirrorStatus> {
  return fetchJson<RemoteMirrorStatus>("/api/media/remote");
}

export async function retryRemoteMirrors(): Promise<{ queued: number }> {
  return fetchJson<{ queued: number }>("/api/media/remote/retry", { method: "POST" });
}

export async function cleanupMedia(): Promise<MediaCleanupResponse> {
  return fetchJson<MediaCleanupResponse>("/api/media/cleanup", { method: "POST" });
}

export async function fetchAgentDiagnostics(): Promise<AgentDiagnosticsResponse> {
  return fetchJson<AgentDiagnosticsResponse>("/api/agent/diagnostics");
}

export async function retryScreenshotOcr(screenshotId: string): Promise<ScreenshotRecord> {
  return fetchJson<ScreenshotRecord>(`/api/screenshots/${encodeURIComponent(screenshotId)}/ocr`, { method: "POST" });
}

async function deleteRequest(url: string): Promise<void> {
  const response = await fetch(url, { method: "DELETE", headers: { accept: "application/json" } });
  if (!response.ok) throw new ApiError(`Failed to delete ${url}: HTTP ${response.status}`, response.status);
  responseCache.clear();
  inflightCache.clear();
}

export async function deleteScreenshot(screenshotId: string): Promise<void> {
  return deleteRequest(`/api/screenshots/${encodeURIComponent(screenshotId)}`);
}

export async function deleteActivity(eventId: string): Promise<void> {
  return deleteRequest(`/api/activities/${encodeURIComponent(eventId)}`);
}

export async function fetchTimeline(filters: TimelineFilters): Promise<TimelineResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value !== undefined && value !== null && String(value).trim() !== "") params.set(key, String(value));
  }
  return fetchJson<TimelineResponse>(`/api/timeline?${params.toString()}`);
}

export async function fetchWorkSessions(filters: Pick<TimelineFilters, "date" | "start" | "end" | "deviceId">): Promise<WorkSessionResponse> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) if (value) params.set(key, String(value));
  return fetchJson<WorkSessionResponse>(`/api/sessions?${params.toString()}`);
}

export async function deleteActivities(filters: Omit<TimelineFilters, "q" | "limit" | "offset"> & { eventId?: string }): Promise<DeletionSummary> {
  const result = await fetchJson<DeletionSummary>("/api/activities/bulk-delete", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(filters)
  });
  responseCache.clear();
  inflightCache.clear();
  return result;
}

export async function setDeviceRecording(deviceId: string, enabled: boolean): Promise<DeviceRecordingState> {
  const state = await fetchJson<DeviceRecordingState>(`/api/devices/${encodeURIComponent(deviceId)}/recording`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ enabled })
  });
  responseCache.clear();
  inflightCache.clear();
  return state;
}

export async function fetchReviewSettings(): Promise<ReviewSettings> {
  return fetchJson<ReviewSettings>("/api/settings/review");
}

export async function saveReviewSettings(settings: ReviewSettings): Promise<ReviewSettings> {
  const saved = await fetchJson<ReviewSettings>("/api/settings/review", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(settings)
  });
  responseCache.clear();
  inflightCache.clear();
  return saved;
}

export async function importReviewSettings(file: File): Promise<ReviewSettings> {
  const saved = await fetchJson<ReviewSettings>("/api/settings/review/import", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: await file.text()
  });
  responseCache.clear();
  inflightCache.clear();
  return saved;
}

export async function fetchReports(start: string, end: string): Promise<DailyReport[]> {
  const params = new URLSearchParams({ start, end });
  return fetchJson<DailyReport[]>(`/api/reports?${params.toString()}`);
}

export async function fetchAssistantConversations(): Promise<AssistantConversation[]> {
  return fetchJson<AssistantConversation[]>("/api/assistant/conversations");
}

export async function fetchAssistantConversation(id: string): Promise<AssistantConversationDetail> {
  return fetchJson<AssistantConversationDetail>(`/api/assistant/conversations/${encodeURIComponent(id)}`);
}

export async function fetchAssistantPrompts(): Promise<string[]> {
  return fetchJson<string[]>("/api/assistant/prompts");
}

export async function deleteAssistantConversation(id: string): Promise<void> {
  return deleteRequest(`/api/assistant/conversations/${encodeURIComponent(id)}`);
}

export async function streamAssistantReply(
  prompt: string,
  conversationId: string | null,
  useAi: boolean,
  onToken: (token: string) => void
): Promise<AssistantReply> {
  const response = await fetch("/api/assistant/stream", {
    method: "POST",
    headers: { accept: "application/x-ndjson", "content-type": "application/json" },
    body: JSON.stringify({ prompt, conversationId, useAi })
  });
  if (!response.ok || !response.body) throw new ApiError(`Assistant request failed: HTTP ${response.status}`, response.status);
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let reply: AssistantReply | null = null;
  while (true) {
    const { value, done } = await reader.read();
    buffer += decoder.decode(value ?? new Uint8Array(), { stream: !done });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";
    for (const line of lines) {
      if (!line.trim()) continue;
      const event = JSON.parse(line) as { type: string; token?: string; error?: string; reply?: AssistantReply };
      if (event.type === "token" && event.token) onToken(event.token);
      if (event.type === "error") throw new Error(event.error || "Assistant request failed");
      if (event.type === "done" && event.reply) reply = event.reply;
    }
    if (done) break;
  }
  if (!reply) throw new Error("Assistant stream ended without a reply");
  return reply;
}

export async function fetchDailyReport(date: string): Promise<DailyReport | null> {
  try {
    return await fetchJson<DailyReport>(`/api/reports/${encodeURIComponent(date)}`);
  } catch (error) {
    if (error instanceof ApiError && error.status === 404) return null;
    throw error;
  }
}

export async function generateDailyReport(date: string, useAi: boolean): Promise<DailyReport> {
  return fetchJson<DailyReport>(`/api/reports/${encodeURIComponent(date)}/generate?useAi=${useAi}`, { method: "POST" });
}

export async function saveDailyReport(date: string, content: string): Promise<DailyReport> {
  return fetchJson<DailyReport>(`/api/reports/${encodeURIComponent(date)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ content })
  });
}

export async function searchMemory(query: string, limit = 40): Promise<MemorySearchResponse> {
  const params = new URLSearchParams({ q: query, limit: String(limit) });
  return fetchJson<MemorySearchResponse>(`/api/memory?${params.toString()}`);
}

export async function reindexMemory(): Promise<MemoryReindexResponse> {
  return fetchJson<MemoryReindexResponse>("/api/memory/reindex", { method: "POST" });
}
