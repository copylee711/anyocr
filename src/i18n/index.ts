import { createI18n } from "vue-i18n";

import en from "./en";
import zhCN from "./zh-CN";

export type MessageSchema = typeof zhCN;

export const i18n = createI18n<[MessageSchema], "zh-CN" | "en">({
  legacy: false,
  locale: navigator.language.startsWith("zh") ? "zh-CN" : "en",
  fallbackLocale: "zh-CN",
  messages: { "zh-CN": zhCN, en },
});
