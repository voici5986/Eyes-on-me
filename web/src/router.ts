import { createRouter, createWebHistory } from "vue-router";
import { fetchAnalysisOverview, fetchAuthSession, fetchDeviceAnalysis, fetchDeviceDetail, fetchDevices } from "./api";
import { DEFAULT_ANALYSIS_RANGE, normalizeAnalysisRange } from "./lib/analysis-range";
import DeviceAnalysisView from "./views/DeviceAnalysisView.vue";
import DeviceDetailView from "./views/DeviceDetailView.vue";
import HomeView from "./views/HomeView.vue";
import MemoryView from "./views/MemoryView.vue";
import ReportsView from "./views/ReportsView.vue";
import MediaView from "./views/MediaView.vue";
import SettingsView from "./views/SettingsView.vue";
import AssistantView from "./views/AssistantView.vue";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      name: "analysis-overview",
      component: HomeView
    },
    {
      path: "/devices/:deviceId",
      name: "device-detail",
      component: DeviceDetailView
    },
    {
      path: "/devices/:deviceId/analysis",
      name: "device-analysis",
      component: DeviceAnalysisView
    },
    {
      path: "/reports",
      name: "reports",
      component: ReportsView
    },
    {
      path: "/assistant",
      name: "assistant",
      component: AssistantView
    },
    {
      path: "/memory",
      name: "memory",
      component: MemoryView
    },
    {
      path: "/media",
      name: "media",
      component: MediaView
    },
    {
      path: "/settings",
      name: "settings",
      component: SettingsView
    }
  ]
});

router.beforeResolve(async (to) => {
  try {
    const session = await fetchAuthSession();
    if (!session.authenticated) return;
    if (to.name === "analysis-overview") {
      await Promise.all([
        fetchDevices(),
        fetchAnalysisOverview(normalizeAnalysisRange(to.query.range))
      ]);
      return;
    }

    if (to.name === "device-detail") {
      const deviceId = String(to.params.deviceId ?? "");
      if (!deviceId) {
        return;
      }

      await Promise.all([
        fetchDeviceDetail(deviceId),
        fetchDeviceAnalysis(deviceId, DEFAULT_ANALYSIS_RANGE)
      ]);
      return;
    }

    if (to.name === "device-analysis") {
      const deviceId = String(to.params.deviceId ?? "");
      if (!deviceId) {
        return;
      }

      await Promise.all([
        fetchDeviceDetail(deviceId),
        fetchDeviceAnalysis(deviceId, normalizeAnalysisRange(to.query.range))
      ]);
    }
  } catch (error) {
    console.warn("route prefetch failed", error);
  }
});
