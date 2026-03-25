import type {
  AnalysisRange,
  AnalysisOverviewResponse,
  DashboardSnapshot,
  DeviceAnalysisResponse,
  DeviceDetailResponse,
  DevicesResponse,
  StreamMessage
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

async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url, {
    headers: {
      accept: "application/json"
    }
  });

  if (!response.ok) {
    throw new ApiError(`Failed to fetch ${url}: HTTP ${response.status}`, response.status);
  }

  return response.json() as Promise<T>;
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
