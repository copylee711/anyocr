import { createRouter, createWebHashHistory, type RouteRecordRaw } from "vue-router";

export interface NavItem {
  path: string;
  /** i18n key */
  label: string;
  icon: string;
}

/** 左侧导航，顺序与 Umi-OCR 的功能页对应。 */
export const navItems: NavItem[] = [
  { path: "/screenshot", label: "nav.screenshot", icon: "scan" },
  { path: "/batch-ocr", label: "nav.batchOcr", icon: "images" },
  { path: "/batch-doc", label: "nav.batchDoc", icon: "document" },
  { path: "/qrcode", label: "nav.qrcode", icon: "qrcode" },
];

export const bottomNavItems: NavItem[] = [
  { path: "/settings", label: "nav.settings", icon: "settings" },
  { path: "/about", label: "nav.about", icon: "info" },
];

const routes: RouteRecordRaw[] = [
  { path: "/", redirect: "/screenshot" },
  { path: "/screenshot", component: () => import("./pages/ScreenshotOcr.vue") },
  { path: "/batch-ocr", component: () => import("./pages/BatchOcr.vue") },
  { path: "/batch-doc", component: () => import("./pages/BatchDoc.vue") },
  { path: "/qrcode", component: () => import("./pages/QrCode.vue") },
  { path: "/settings", component: () => import("./pages/Settings.vue") },
  { path: "/about", component: () => import("./pages/About.vue") },
  { path: "/overlay", component: () => import("./pages/Overlay.vue") },
];

export const router = createRouter({ history: createWebHashHistory(), routes });
