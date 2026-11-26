//! 视觉大模型命令：服务预设、测试连接、对单张图片的识别结果做复核（截图页使用）。

use std::time::Instant;

use anyocr_core::review::{self, ReviewOutcome, Reviewer};
use anyocr_core::{Guard, PageResult, TextBlock};
use anyocr_engines::vlm::{self, VlmPreset, VlmReviewer};
use serde::Serialize;
use tauri::State;

use crate::engines;
use crate::settings::SettingsState;

#[tauri::command]
pub fn vlm_presets() -> Vec<VlmPreset> {
    vlm::presets()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VlmTestResult {
    ok: bool,
    elapsed_ms: u64,
    /// 测试样张中故意写错的行被改正的条数（共 2 处）。
    fixed: usize,
    model: String,
    error: Option<String>,
}

/// 用内置样张测试：给出两处故意写错的 OCR 结果，看模型能否改回。
#[tauri::command]
pub async fn vlm_test(settings: State<'_, SettingsState>) -> Result<VlmTestResult, String> {
    let v = settings.vlm();
    tauri::async_runtime::spawn_blocking(move || {
        let start = Instant::now();
        let img = image::load_from_memory(include_bytes!("../../tests/fixtures/hello.png"))
            .map_err(|e| e.to_string())?
            .to_rgb8();
        let page = PageResult {
            width: img.width(),
            height: img.height(),
            blocks: vec![
                TextBlock::from_rect("AnyOCR online engine test", [30., 25., 560., 75.], 0.99),
                TextBlock::from_rect(
                    "Recognize anyth1ng. Own your dafa.",
                    [30., 90., 620., 130.],
                    0.8,
                ),
                TextBlock::from_rect("识别任何文宇 2026", [30., 150., 360., 200.], 0.7),
            ],
            ..Default::default()
        };
        let r = engines::vlm_client(&v).and_then(|client| {
            let model = client.model().to_owned();
            VlmReviewer::new(client, Guard::default())
                .review(&img, &page)
                .map(|o| (o, model))
                .map_err(|e| e.to_string())
        });
        let elapsed_ms = start.elapsed().as_millis() as u64;
        Ok(match r {
            Ok((o, model)) => VlmTestResult {
                ok: true,
                elapsed_ms,
                fixed: o.accepted(),
                model,
                error: None,
            },
            Err(e) => VlmTestResult {
                ok: false,
                elapsed_ms,
                fixed: 0,
                model: String::new(),
                error: Some(e),
            },
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewResult {
    outcome: ReviewOutcome,
    /// 应用修改后的页面与文本。
    page: PageResult,
    text: String,
    /// 是否已用新文本替换剪贴板。
    recopied: bool,
}

/// 复核一张图片的识别结果。`recopy` 为识别后已复制的原文：剪贴板内容仍是它时，替换为复核后的文本。
#[tauri::command]
pub async fn review_image(
    settings: State<'_, SettingsState>,
    path: String,
    page: PageResult,
    recopy: Option<String>,
) -> Result<ReviewResult, String> {
    let v = settings.vlm();
    tauri::async_runtime::spawn_blocking(move || {
        let img = image::open(&path)
            .map_err(|e| format!("无法打开图片：{e}"))?
            .to_rgb8();
        let client = engines::vlm_client(&v)?;
        let outcome = VlmReviewer::new(client, Guard::default())
            .review(&img, &page)
            .map_err(|e| e.to_string())?;
        let mut page = page;
        review::apply(&mut page, &outcome);
        let text = anyocr_output::data_text(&page);
        let mut recopied = false;
        if let Some(orig) = recopy
            && outcome.accepted() > 0
            && let Ok(mut cb) = arboard::Clipboard::new()
            && cb.get_text().is_ok_and(|t| t == orig)
        {
            recopied = cb.set_text(text.clone()).is_ok();
        }
        Ok(ReviewResult {
            outcome,
            page,
            text,
            recopied,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
