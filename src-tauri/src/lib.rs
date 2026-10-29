use std::cell::Cell;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::BufReader;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use jpeg_encoder::{ColorType, Encoder};
use serde::Serialize;
use tauri::{Manager, ipc::Channel};
use zune_heic::HeifDecoder;

thread_local! {
    static DECODER_PANIC_IS_HANDLED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum ConversionEvent {
    Scanned {
        total: usize,
    },
    Progress {
        completed: usize,
        total: usize,
        succeeded: usize,
        failed: usize,
        #[serde(rename = "fileName")]
        file_name: String,
        error: Option<String>,
    },
    Finished {
        succeeded: usize,
        failed: usize,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversionSummary {
    succeeded: usize,
    failed: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultDirectories {
    input_dir: String,
    output_dir: String,
}

#[tauri::command]
fn default_directories(app: tauri::AppHandle) -> DefaultDirectories {
    let pictures_dir = app
        .path()
        .picture_dir()
        .or_else(|_| std::env::current_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    DefaultDirectories {
        input_dir: pictures_dir.display().to_string(),
        output_dir: pictures_dir.join("HEIC Converted").display().to_string(),
    }
}

#[tauri::command]
async fn convert_images(
    input_dir: String,
    output_dir: String,
    quality: u8,
    on_event: Channel<ConversionEvent>,
) -> std::result::Result<ConversionSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_conversion(
            PathBuf::from(input_dir),
            PathBuf::from(output_dir),
            quality,
            on_event,
        )
        .map_err(|error| format!("{error:#}"))
    })
    .await
    .map_err(|error| format!("轉換工作意外中止：{error}"))?
}

fn run_conversion(
    input_dir: PathBuf,
    output_dir: PathBuf,
    quality: u8,
    on_event: Channel<ConversionEvent>,
) -> Result<ConversionSummary> {
    if !input_dir.is_dir() {
        bail!("請選擇有效的來源資料夾。");
    }
    if output_dir.as_os_str().is_empty() {
        bail!("請選擇輸出資料夾。");
    }
    if !(60..=100).contains(&quality) {
        bail!("JPEG 品質必須介於 60 到 100 之間。");
    }

    let mut files = Vec::new();
    scan_heic_files(&input_dir, &mut files).context("掃描資料夾失敗")?;
    files.sort_unstable();
    let total = files.len();
    send_event(&on_event, ConversionEvent::Scanned { total })?;

    let mut succeeded = 0;
    let mut failed = 0;
    for (index, input_path) in files.into_iter().enumerate() {
        let file_name = input_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| input_path.display().to_string());

        let result = input_path
            .strip_prefix(&input_dir)
            .context("無法計算相對路徑")
            .and_then(|relative_path| {
                let output_path = output_dir.join(relative_path).with_extension("jpg");
                catch_decoder_panic(|| convert_heic_to_jpeg(&input_path, &output_path, quality))
            });

        let error = match result {
            Ok(()) => {
                succeeded += 1;
                None
            }
            Err(error) => {
                failed += 1;
                Some(format!("{error:#}"))
            }
        };

        send_event(
            &on_event,
            ConversionEvent::Progress {
                completed: index + 1,
                total,
                succeeded,
                failed,
                file_name,
                error,
            },
        )?;
    }

    send_event(&on_event, ConversionEvent::Finished { succeeded, failed })?;
    Ok(ConversionSummary { succeeded, failed })
}

fn send_event(channel: &Channel<ConversionEvent>, event: ConversionEvent) -> Result<()> {
    channel
        .send(event)
        .map_err(|error| anyhow!("無法更新前端進度：{error}"))
}

fn scan_heic_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("無法讀取資料夾：{}", directory.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("無法讀取：{}", directory.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("無法取得檔案類型：{}", entry.path().display()))?;
        if file_type.is_dir() {
            scan_heic_files(&entry.path(), files)?;
        } else if file_type.is_file() && is_heic(entry.path().extension()) {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn is_heic(extension: Option<&OsStr>) -> bool {
    extension.and_then(OsStr::to_str).is_some_and(|extension| {
        extension.eq_ignore_ascii_case("heic") || extension.eq_ignore_ascii_case("heif")
    })
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let is_handled = DECODER_PANIC_IS_HANDLED.with(Cell::get);
        if !is_handled {
            default_hook(panic_info);
        }
    }));
}

fn catch_decoder_panic<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    DECODER_PANIC_IS_HANDLED.with(|flag| flag.set(true));
    let result = panic::catch_unwind(AssertUnwindSafe(operation));
    DECODER_PANIC_IS_HANDLED.with(|flag| flag.set(false));

    match result {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("未知的解碼器錯誤");
            bail!("HEIC 解碼器無法處理此檔案（檔案可能損壞或使用尚未支援的格式）：{message}")
        }
    }
}

fn convert_heic_to_jpeg(input_path: &Path, output_path: &Path, quality: u8) -> Result<()> {
    let input = File::open(input_path)
        .with_context(|| format!("無法開啟輸入檔案：{}", input_path.display()))?;
    let mut decoder = HeifDecoder::new(BufReader::new(input));
    let pixels = decoder.decode().context("HEIC 解碼失敗")?;
    let width = decoder.width().context("解碼器沒有回傳圖片寬度")?;
    let height = decoder.height().context("解碼器沒有回傳圖片高度")?;
    let components = decoder
        .colorspace()
        .context("解碼器沒有回傳色彩格式")?
        .num_components();

    let (jpeg_pixels, color_type) = jpeg_compatible_pixels(pixels, components)?;
    let width = u16::try_from(width).context("圖片寬度超過 JPEG 編碼器限制")?;
    let height = u16::try_from(height).context("圖片高度超過 JPEG 編碼器限制")?;
    let mut encoded = Vec::new();
    Encoder::new(&mut encoded, quality)
        .encode(&jpeg_pixels, width, height, color_type)
        .context("JPEG 編碼失敗")?;

    let parent = output_path
        .parent()
        .ok_or_else(|| anyhow!("輸出路徑沒有上層資料夾：{}", output_path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("無法建立輸出資料夾：{}", parent.display()))?;
    fs::write(output_path, encoded)
        .with_context(|| format!("無法寫入輸出檔案：{}", output_path.display()))?;
    Ok(())
}

fn jpeg_compatible_pixels(pixels: Vec<u8>, components: usize) -> Result<(Vec<u8>, ColorType)> {
    match components {
        1 => Ok((pixels, ColorType::Luma)),
        2 => Ok((
            pixels
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pixel| pixel[0])
                .collect(),
            ColorType::Luma,
        )),
        3 => Ok((pixels, ColorType::Rgb)),
        4 => Ok((
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                .collect(),
            ColorType::Rgb,
        )),
        _ => bail!("不支援每像素 {components} 個色彩通道的圖片"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            default_directories,
            convert_images
        ])
        .run(tauri::generate_context!())
        .expect("error while running HEIC Converter");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_heic_extensions_case_insensitively() {
        assert!(is_heic(Some(OsStr::new("heic"))));
        assert!(is_heic(Some(OsStr::new("HEIF"))));
        assert!(!is_heic(Some(OsStr::new("jpg"))));
        assert!(!is_heic(None));
    }

    #[test]
    fn removes_alpha_before_jpeg_encoding() {
        let (pixels, color_type) = jpeg_compatible_pixels(vec![1, 2, 3, 4, 5, 6, 7, 8], 4).unwrap();
        assert_eq!(pixels, vec![1, 2, 3, 5, 6, 7]);
        assert_eq!(color_type, ColorType::Rgb);
    }

    #[test]
    fn decoder_panic_becomes_a_normal_error() {
        let result = catch_decoder_panic(|| -> Result<()> {
            panic!("unsupported HEVC bitstream");
        });

        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("HEIC 解碼器無法處理此檔案"));
        assert!(error.contains("unsupported HEVC bitstream"));
    }

    #[test]
    fn progress_event_serializes_file_name_for_the_frontend() {
        let event = ConversionEvent::Progress {
            completed: 1,
            total: 1,
            succeeded: 1,
            failed: 0,
            file_name: "photo.heic".to_owned(),
            error: None,
        };

        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["fileName"], "photo.heic");
        assert!(json.get("file_name").is_none());
    }
}
