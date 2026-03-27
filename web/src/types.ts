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
}

export interface DevicesResponse {
  devices: DeviceOverview[];
}

export interface DeviceDetailResponse {
  device: ActivityEvent;
  latestStatus: DeviceStatus | null;
  recentActivities: ActivityEvent[];
}

export interface UsageBucket {
  key: string;
  label: string;
  sublabel?: string | null;
  totalTrackedMs: number;
  sessions: number;
  lastSeen: string;
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
  devices: DeviceAnalysisSummary[];
  topAppUsage: UsageBucket[];
  topDomainUsage: UsageBucket[];
  topBrowserUsage: BrowserUsageBucket[];
}

export interface DeviceAnalysisResponse {
  deviceId: string;
  generatedAt: string;
  totalTrackedMs: number;
  eventCount: number;
  currentLabel?: string | null;
  latestStatus: DeviceStatus | null;
  appUsage: UsageBucket[];
  domainUsage: UsageBucket[];
  browserUsage: BrowserUsageBucket[];
}

export interface StreamMessage<T = unknown> {
  type: "snapshot" | "ping";
  payload: T;
}
