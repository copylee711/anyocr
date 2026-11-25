# AnyOCR

本地优先的 OCR 桌面应用，使用 Tauri 2 + Vue 3 重写 [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR)，并新增：

- **在线 OCR 引擎**：PaddleOCR（AI Studio）、MinerU 等，在设置页填写自己的 API Key 即可使用。
- **视觉大模型**：可以直接用来识别，也可以与 OCR 引擎并行运行，二次复核并修正识别结果，尽量不增加总用时。
- **扫描件 PDF 加文本层**：参考 OCRmyPDF，为扫描版 PDF 添加不可见文字层，使其支持搜索和复制，原图像不重新压缩。

> 开发中。计划见 [docs/PLAN.md](docs/PLAN.md)，调研见 [docs/RESEARCH.md](docs/RESEARCH.md)。

## 致谢

- [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR)（MIT）：功能设计与部分算法来源，详见 [NOTICE](NOTICE)。
- [OCRmyPDF](https://github.com/ocrmypdf/OCRmyPDF)：PDF 文字层方案参考。
- [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR)、[oar-ocr](https://github.com/greatv/oar-ocr)、[pdfium-render](https://github.com/ajrcarey/pdfium-render)。
