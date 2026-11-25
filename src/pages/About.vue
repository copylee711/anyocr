<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const credits = [
  { name: "Umi-OCR", license: "MIT", url: "https://github.com/hiroi-sora/Umi-OCR" },
  { name: "OCRmyPDF", license: "MPL-2.0", url: "https://github.com/ocrmypdf/OCRmyPDF" },
  { name: "PaddleOCR", license: "Apache-2.0", url: "https://github.com/PaddlePaddle/PaddleOCR" },
  { name: "oar-ocr", license: "Apache-2.0", url: "https://github.com/greatv/oar-ocr" },
  { name: "PDFium", license: "BSD-3-Clause", url: "https://pdfium.googlesource.com/pdfium/" },
  { name: "Tauri", license: "MIT / Apache-2.0", url: "https://tauri.app" },
];
</script>

<template>
  <section class="page">
    <header class="hero">
      <img src="/logo.svg" alt="" />
      <div>
        <h1>AnyOCR</h1>
        <p class="slogan">Recognize anything. Own your data.</p>
        <p>{{ t("about.desc") }}</p>
      </div>
    </header>
    <div class="card list">
      <h3>{{ t("about.credits") }}</h3>
      <a v-for="c in credits" :key="c.name" class="item" href="#" @click.prevent="openUrl(c.url)">
        <span>{{ c.name }}</span>
        <small>{{ c.license }}</small>
      </a>
    </div>
  </section>
</template>

<style scoped>
.hero {
  display: flex;
  gap: 20px;
  align-items: center;
  max-width: 720px;
  padding: 8px 0 6px;
}
.hero img {
  width: 96px;
  height: 96px;
  border-radius: 22px;
  box-shadow: 0 12px 30px -14px rgba(0, 0, 0, 0.5);
}
.hero h1 {
  margin: 0;
  font-size: 30px;
  font-weight: 700;
  letter-spacing: -0.02em;
}
.hero p {
  margin: 4px 0 0;
  color: var(--c-text-2);
}
.hero .slogan {
  color: var(--c-text);
  font-size: 16px;
}
.list {
  padding: 16px 18px;
  max-width: 560px;
}
h3 {
  margin: 0 0 8px;
  font-size: 14px;
  color: var(--c-text-2);
  font-weight: 600;
}
.item {
  display: flex;
  justify-content: space-between;
  padding: 10px 2px;
  color: var(--c-text);
  text-decoration: none;
  border-top: 1px solid var(--c-border);
}
.item:hover span {
  color: var(--c-primary);
}
small {
  color: var(--c-text-3);
}
</style>
