import type { GlobalThemeOverrides } from "naive-ui";

// 仿 iOS 的 Naive UI 主题覆盖，与 src/styles/base.css 的令牌保持一致。
const shared: GlobalThemeOverrides["common"] = {
  fontFamily:
    '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", "PingFang SC", "Microsoft YaHei UI", system-ui, sans-serif',
  fontFamilyMono: '"SF Mono", "Cascadia Code", "JetBrains Mono", Consolas, monospace',
  borderRadius: "10px",
  borderRadiusSmall: "8px",
};

export const lightOverrides: GlobalThemeOverrides = {
  common: {
    ...shared,
    primaryColor: "#007AFF",
    primaryColorHover: "#2B8FFF",
    primaryColorPressed: "#0066D6",
    primaryColorSuppl: "#2B8FFF",
    infoColor: "#32ADE6",
    successColor: "#34C759",
    warningColor: "#FF9500",
    errorColor: "#FF3B30",
    bodyColor: "#F2F2F7",
    cardColor: "#FFFFFF",
    borderColor: "rgba(60, 60, 67, 0.12)",
    textColorBase: "#000000",
  },
  Button: { borderRadiusMedium: "10px", fontWeight: "600" },
};

export const darkOverrides: GlobalThemeOverrides = {
  common: {
    ...shared,
    primaryColor: "#0A84FF",
    primaryColorHover: "#3399FF",
    primaryColorPressed: "#0070DB",
    primaryColorSuppl: "#3399FF",
    infoColor: "#64D2FF",
    successColor: "#30D158",
    warningColor: "#FF9F0A",
    errorColor: "#FF453A",
    bodyColor: "#000000",
    cardColor: "#1C1C1E",
    modalColor: "#1C1C1E",
    popoverColor: "#2C2C2E",
    borderColor: "rgba(84, 84, 88, 0.6)",
  },
  Button: { borderRadiusMedium: "10px", fontWeight: "600" },
};
