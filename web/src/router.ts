import { createRouter, createWebHistory } from "vue-router";
import { fetchAnalysisOverview, fetchDeviceAnalysis, fetchDeviceDetail, fetchDevices } from "./api";
import { DEFAULT_ANALYSIS_RANGE, normalizeAnalysisRange } from "./lib/analysis-range";
import DeviceAnalysisView from "./views/DeviceAnalysisView.vue";
import DeviceDetailView from "./views/DeviceDetailView.vue";
import HomeView from "./views/HomeView.vue";

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
    }
  ]
});

router.beforeResolve(async (to) => {
  try {
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
