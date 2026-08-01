export interface ActivityApp {
  id: string;
  name: string;
  title?: string | null;
  pid?: number | null;
}

export interface BrowserContext {
  family: string;
  name: string;
  pageTitle?: string | null;
  url?: string | null;
  domain?: string | null;
  source: string;
  confidence: number;
}

export type PresenceState = "active" | "idle" | "locked";

export interface ActivityEvent {
  eventId: string;
  ts: string;
  deviceId: string;
  agentName: string;
  platform: string;
  kind: string;
  app: ActivityApp;
  windowTitle?: string | null;
  browser?: BrowserContext | null;
  presence: PresenceState;
  source: string;
}

export interface DeviceStatus {
  ts: string;
  deviceId: string;
  agentName: string;
  platform: string;
  statusText: string;
  source: string;
}

export interface DashboardSnapshot {
  devices: ActivityEvent[];
  latestStatus: DeviceStatus | null;
  recentActivities: ActivityEvent[];
}

export interface DeviceOverview {
  device: ActivityEvent;
  latestStatus: DeviceStatus | null;
  recording: DeviceRecordingState;
}

export interface DevicesResponse {
  devices: DeviceOverview[];
}

export interface DeviceDetailResponse {
  device: ActivityEvent;
  latestStatus: DeviceStatus | null;
  recentActivities: ActivityEvent[];
  recording: DeviceRecordingState;
}

export interface DeviceRecordingState {
  deviceId: string;
  desiredEnabled: boolean;
  appliedEnabled?: boolean | null;
  revision: number;
  updatedAt: string;
  acknowledgedAt?: string | null;
}

export interface UsageBucket {
  key: string;
  label: string;
  sublabel?: string | null;
  totalTrackedMs: number;
  sessions: number;
  lastSeen: string;
}

export interface AppUsageBucket extends UsageBucket {
  windows: UsageBucket[];
}

export interface CategoryUsageBucket {
  key: string;
  label: string;
  totalTrackedMs: number;
}

export interface HourlyUsageBucket {
  hour: number;
  totalTrackedMs: number;
  apps: UsageBucket[];
}

export interface DailyUsageBucket {
  date: string;
  totalTrackedMs: number;
  workTrackedMs: number;
  browserTrackedMs: number;
}

export interface PageUsageBucket {
  key: string;
  label: string;
  url?: string | null;
  totalTrackedMs: number;
  sessions: number;
  lastSeen: string;
}

export interface DomainUsageBucket {
  key: string;
  label: string;
  totalTrackedMs: number;
  sessions: number;
  lastSeen: string;
  pages: PageUsageBucket[];
}

export interface BrowserUsageBucket {
  key: string;
  label: string;
  family: string;
  totalTrackedMs: number;
  sessions: number;
  lastSeen: string;
  domains: DomainUsageBucket[];
}

export type AnalysisRange = "3h" | "6h" | "today" | "1d" | "1w" | "1m" | "all";

export interface DeviceAnalysisSummary {
  deviceId: string;
  platform: string;
  currentLabel: string;
  latestStatusText?: string | null;
  totalTrackedMs: number;
  eventCount: number;
  lastSeen: string;
}

export interface AnalysisOverviewResponse {
  generatedAt: string;
  deviceCount: number;
  totalTrackedMs: number;
  workTrackedMs: number;
  afterHoursTrackedMs: number;
  browserTrackedMs: number;
  idleTrackedMs: number;
  lockedTrackedMs: number;
  appCount: number;
  devices: DeviceAnalysisSummary[];
  topAppUsage: AppUsageBucket[];
  topDomainUsage: UsageBucket[];
  topBrowserUsage: BrowserUsageBucket[];
  categoryUsage: CategoryUsageBucket[];
  hourlyUsage: HourlyUsageBucket[];
  dailyUsage: DailyUsageBucket[];
}

export interface DeviceAnalysisResponse {
  deviceId: string;
  generatedAt: string;
  totalTrackedMs: number;
  workTrackedMs: number;
  afterHoursTrackedMs: number;
  browserTrackedMs: number;
  idleTrackedMs: number;
  lockedTrackedMs: number;
  appCount: number;
  eventCount: number;
  currentLabel?: string | null;
  latestStatus: DeviceStatus | null;
  appUsage: AppUsageBucket[];
  domainUsage: UsageBucket[];
  browserUsage: BrowserUsageBucket[];
  categoryUsage: CategoryUsageBucket[];
  hourlyUsage: HourlyUsageBucket[];
  dailyUsage: DailyUsageBucket[];
}

export interface ActivitySearchHit {
  activity: ActivityEvent;
  snippet?: string | null;
  score: number;
}

export interface ActivitySearchResponse {
  query: string;
  deviceId?: string | null;
  total: number;
  results: ActivitySearchHit[];
}

export interface AuthSession {
  authenticated: boolean;
  authRequired: boolean;
}

export interface ScreenshotRecord {
  id: string;
  eventId: string;
  deviceId: string;
  capturedAt: string;
  mimeType: string;
  byteSize: number;
  width?: number | null;
  height?: number | null;
  sha256: string;
  ocrStatus: "pending" | "complete" | "failed" | "unavailable" | string;
  ocrText?: string | null;
  ocrError?: string | null;
  ocrAttempts: number;
  duplicateOf?: string | null;
  contentUrl: string;
  thumbnailUrl: string;
  createdAt: string;
}

export interface MediaStatus {
  screenshotCount: number;
  totalBytes: number;
  thumbnailBytes: number;
  totalLimitBytes: number;
  retentionDays: number;
  pendingOcr: number;
  failedOcr: number;
  oldestCapture?: string | null;
  newestCapture?: string | null;
}

export interface MediaCleanupResponse {
  deletedScreenshots: number;
  freedBytes: number;
  status: MediaStatus;
}

export interface RemoteMirrorStatus {
  configured: boolean;
  provider?: string | null;
  pending: number;
  complete: number;
  failed: number;
  lastError?: string | null;
}

export interface AgentDiagnostics {
  deviceId: string;
  agentName: string;
  platform: string;
  updatedAt: string;
  accessibilityPermission: string;
  screenCapturePermission: string;
  screenshotEnabled: boolean;
  screenshotFormat: string;
  screenshotDisplay: string;
  privacyRuleCount: number;
  spoolPending: number;
  recordingEnabled: boolean;
  controlRevision: number;
}

export interface AgentDiagnosticsResponse { agents: AgentDiagnostics[]; }

export interface ScreenshotListResponse {
  screenshots: ScreenshotRecord[];
}

export interface DailyReport {
  date: string;
  content: string;
  generationMode: string;
  modelName?: string | null;
  fallbackReason?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryEntry {
  id: string;
  date: string;
  sourceType: string;
  title: string;
  content: string;
  tags: string[];
  score?: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface MemorySearchResponse {
  query: string;
  mode: "semantic" | "full_text" | "recent" | string;
  entries: MemoryEntry[];
}

export interface MemoryReindexResponse {
  indexedEntries: number;
  embeddedEntries: number;
  embeddingFailures: number;
}

export type CategoryTarget = "app" | "domain";

export interface CategoryRule {
  id: string;
  name: string;
  color: string;
  target: CategoryTarget;
  pattern: string;
  priority: number;
}

export interface WorkScheduleSegment {
  id: string;
  weekday: number;
  startMinute: number;
  endMinute: number;
}

export interface ReportPreferences {
  pinnedBlocks: string[];
  hiddenBlocks: string[];
  blockOrder: string[];
  autoExportEnabled: boolean;
  autoExportDirectory?: string | null;
}

export interface ReviewSettings {
  categoryRules: CategoryRule[];
  workSchedule: WorkScheduleSegment[];
  reportPreferences: ReportPreferences;
}

export interface TimelineItem {
  activity: ActivityEvent;
  screenshot?: ScreenshotRecord | null;
  durationMs: number;
  categoryKey: string;
  categoryLabel: string;
}

export interface TimelineResponse {
  items: TimelineItem[];
  total: number;
  limit: number;
  offset: number;
  start: string;
  end: string;
}

export interface TimelineFilters {
  date?: string;
  start?: string;
  end?: string;
  deviceId?: string;
  app?: string;
  domain?: string;
  category?: string;
  q?: string;
  limit?: number;
  offset?: number;
}

export interface DeletionSummary {
  deletedActivities: number;
  deletedScreenshots: number;
  affectedDates: string[];
}

export interface PotentialTodo {
  text: string;
  sourceEventId: string;
  observedAt: string;
}

export interface WorkSession {
  id: string;
  deviceId: string;
  startedAt: string;
  endedAt: string;
  totalTrackedMs: number;
  appNames: string[];
  summary: string;
  potentialTodos: PotentialTodo[];
}

export interface WorkSessionResponse { sessions: WorkSession[]; }

export interface AssistantConversation {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
}

export interface AssistantMessage {
  id: string;
  conversationId: string;
  role: "user" | "assistant" | string;
  content: string;
  mode: string;
  createdAt: string;
}

export interface AssistantConversationDetail {
  conversation: AssistantConversation;
  messages: AssistantMessage[];
}

export interface AssistantReply {
  conversation: AssistantConversation;
  message: AssistantMessage;
  starterPrompts: string[];
}

export interface StreamMessage<T = unknown> {
  type: "snapshot" | "ping";
  payload: T;
}
