import type zhCN from "./zh-CN";

const en: typeof zhCN = {
  app: { name: "AnyOCR", tagline: "Local-first text recognition" },
  nav: {
    screenshot: "Screenshot",
    batchOcr: "Images",
    batchDoc: "Docs / PDF",
    qrcode: "QR Code",
    settings: "Settings",
    about: "About",
  },
  common: {
    comingSoon: "Coming soon",
    milestone: "Planned for {m}",
    copy: "Copy",
    copied: "Copied",
    elapsed: "{ms} ms",
    blocks: "{n} blocks",
  },
  screenshot: {
    title: "Screenshot OCR",
    desc: "Capture with a hotkey, paste, or open an image to recognize text instantly.",
    open: "Open image",
    empty: "Open an image to start",
    loadingEngine: "Loading local engine (models download on first use)…",
    recognizing: "Recognizing…",
    parser: "Layout",
  },
  batchOcr: { title: "Batch Images", desc: "Recognize folders of images and export txt / md / csv / jsonl." },
  batchDoc: { title: "Docs / PDF", desc: "Add a searchable text layer to scanned PDFs, or export text and Markdown." },
  qrcode: { title: "QR Code", desc: "Read and generate QR codes and barcodes." },
  settings: { title: "Settings", desc: "Engines, online API keys, vision LLMs, appearance and hotkeys." },
  about: {
    title: "About",
    desc: "AnyOCR rewrites Umi-OCR with Tauri + Vue, adding online engines, vision-LLM review and PDF text layers.",
    credits: "Credits",
  },
  parser: {
    multi_para: "Multi-column · paragraphs",
    multi_line: "Multi-column · line breaks",
    multi_none: "Multi-column · no breaks",
    single_para: "Single column · paragraphs",
    single_line: "Single column · line breaks",
    single_none: "Single column · no breaks",
    single_code: "Single column · keep indent",
    none: "No processing",
  },
};

export default en;
