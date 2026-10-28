#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};
use jpeg_encoder::{ColorType, Encoder};
use rfd::{FileDialog, MessageDialog, MessageLevel};
use zune_heic::HeifDecoder;

const DEFAULT_JPEG_QUALITY: u8 = 100;
const ACCENT: Color32 = Color32::from_rgb(38, 132, 255);

fn main() {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("HEIC Converter")
            .with_inner_size([720.0, 560.0])
            .with_min_inner_size([620.0, 500.0]),
        ..Default::default()
    };

    if let Err(error) = eframe::run_native(
        "HEIC Converter",
        options,
        Box::new(|context| Ok(Box::new(ConverterApp::new(context)))),
    ) {
        MessageDialog::new()
            .set_level(MessageLevel::Error)
            .set_title("HEIC Converter")
            .set_description(format!("無法啟動程式：{error}"))
            .show();
    }
}

struct ConverterApp {
    input_dir: String,
    output_dir: String,
    quality: u8,
    state: AppState,
    receiver: Option<Receiver<WorkerMessage>>,
    log: Vec<String>,
}

#[derive(Default)]
enum AppState {
    #[default]
    Idle,
    Running {
        completed: usize,
        total: usize,
        current_file: String,
    },
    Finished {
        succeeded: usize,
        failed: usize,
    },
    Error(String),
}

enum WorkerMessage {
    Scanned(usize),
    Progress {
        completed: usize,
        total: usize,
        succeeded: usize,
        failed: usize,
        file_name: String,
        error: Option<String>,
    },
    Finished {
        succeeded: usize,
        failed: usize,
    },
    Fatal(String),
}

impl ConverterApp {
    fn new(context: &eframe::CreationContext<'_>) -> Self {
        configure_style(&context.egui_ctx);
        load_chinese_font(&context.egui_ctx);

        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            input_dir: current_dir.join("input").display().to_string(),
            output_dir: current_dir.join("output").display().to_string(),
            quality: DEFAULT_JPEG_QUALITY,
            state: AppState::Idle,
            receiver: None,
            log: Vec::new(),
        }
    }

    fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running { .. })
    }

    fn select_input_folder(&mut self) {
        let mut dialog = FileDialog::new().set_title("選擇 HEIC 來源資料夾");
        if Path::new(&self.input_dir).exists() {
            dialog = dialog.set_directory(&self.input_dir);
        }
        if let Some(path) = dialog.pick_folder() {
            self.input_dir = path.display().to_string();
        }
    }

    fn select_output_folder(&mut self) {
        let mut dialog = FileDialog::new().set_title("選擇 JPEG 輸出資料夾");
        if Path::new(&self.output_dir).exists() {
            dialog = dialog.set_directory(&self.output_dir);
        }
        if let Some(path) = dialog.pick_folder() {
            self.output_dir = path.display().to_string();
        }
    }

    fn start(&mut self, context: &egui::Context) {
        let input_dir = PathBuf::from(self.input_dir.trim());
        let output_dir = PathBuf::from(self.output_dir.trim());

        if !input_dir.is_dir() {
            self.state = AppState::Error("請選擇有效的來源資料夾。".to_owned());
            return;
        }
        if self.output_dir.trim().is_empty() {
            self.state = AppState::Error("請選擇輸出資料夾。".to_owned());
            return;
        }

        let quality = self.quality;
        let (sender, receiver) = mpsc::channel();
        let repaint_context = context.clone();
        self.receiver = Some(receiver);
        self.log.clear();
        self.log.push("正在搜尋 HEIC/HEIF 圖片……".to_owned());
        self.state = AppState::Running {
            completed: 0,
            total: 0,
            current_file: "掃描資料夾中".to_owned(),
        };

        thread::spawn(move || {
            run_conversion(input_dir, output_dir, quality, sender, repaint_context);
        });
    }

    fn receive_worker_messages(&mut self) {
        let mut messages = Vec::new();
        if let Some(receiver) = &self.receiver {
            messages.extend(receiver.try_iter());
        }

        for message in messages {
            match message {
                WorkerMessage::Scanned(total) => {
                    if let AppState::Running {
                        completed,
                        total: state_total,
                        current_file,
                    } = &mut self.state
                    {
                        *completed = 0;
                        *state_total = total;
                        *current_file = if total == 0 {
                            "沒有找到可轉換的圖片".to_owned()
                        } else {
                            "準備開始轉換".to_owned()
                        };
                    }
                    self.log.push(format!("找到 {total} 張圖片。"));
                }
                WorkerMessage::Progress {
                    completed,
                    total,
                    succeeded,
                    failed,
                    file_name,
                    error,
                } => {
                    self.state = AppState::Running {
                        completed,
                        total,
                        current_file: file_name.clone(),
                    };
                    if let Some(error) = error {
                        self.log.push(format!("失敗｜{file_name}｜{error}"));
                    } else {
                        self.log.push(format!(
                            "完成｜{file_name}（成功 {succeeded}、失敗 {failed}）"
                        ));
                    }
                }
                WorkerMessage::Finished { succeeded, failed } => {
                    self.state = AppState::Finished { succeeded, failed };
                    self.receiver = None;
                    self.log.push(if succeeded + failed == 0 {
                        "來源資料夾內沒有 .heic 或 .heif 圖片。".to_owned()
                    } else {
                        format!("轉換完成：成功 {succeeded} 張，失敗 {failed} 張。")
                    });
                }
                WorkerMessage::Fatal(error) => {
                    self.state = AppState::Error(error.clone());
                    self.receiver = None;
                    self.log.push(format!("錯誤｜{error}"));
                }
            }
        }
    }

    fn path_row(ui: &mut egui::Ui, label: &str, path: &mut String, enabled: bool) -> bool {
        let mut browse_clicked = false;
        ui.label(RichText::new(label).strong());
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(path).desired_width(f32::INFINITY),
            );
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new("瀏覽…").min_size([76.0, 30.0].into()),
                )
                .clicked()
            {
                browse_clicked = true;
            }
        });
        browse_clicked
    }
}

impl eframe::App for ConverterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        self.receive_worker_messages();
        if self.is_running() {
            context.request_repaint_after(Duration::from_millis(100));
        }

        {
            ui.add_space(8.0);
            ui.heading(RichText::new("HEIC 轉 JPG").size(28.0).strong());
            ui.label(
                RichText::new("快速、離線地批次轉換照片，並保留原有資料夾結構。")
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(18.0);

            let enabled = !self.is_running();
            egui::Frame::group(ui.style())
                .inner_margin(18.0)
                .corner_radius(12.0)
                .show(ui, |ui| {
                    if Self::path_row(ui, "來源資料夾", &mut self.input_dir, enabled) {
                        self.select_input_folder();
                    }
                    ui.add_space(14.0);
                    if Self::path_row(ui, "輸出資料夾", &mut self.output_dir, enabled) {
                        self.select_output_folder();
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("JPEG 品質").strong());
                        ui.add_enabled(
                            enabled,
                            egui::Slider::new(&mut self.quality, 60..=100).suffix("%"),
                        );
                    });
                });

            ui.add_space(14.0);
            show_status(ui, &self.state);
            ui.add_space(14.0);

            let button = egui::Button::new(RichText::new("開始轉換").size(17.0).strong())
                .fill(ACCENT)
                .min_size([160.0, 42.0].into());
            if ui.add_enabled(enabled, button).clicked() {
                self.start(&context);
            }

            ui.add_space(14.0);
            ui.label(RichText::new("處理紀錄").strong());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(120.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.log.is_empty() {
                        ui.label(
                            RichText::new("尚未開始轉換。").color(ui.visuals().weak_text_color()),
                        );
                    } else {
                        for line in &self.log {
                            ui.label(line);
                        }
                    }
                });
        }
    }
}

fn show_status(ui: &mut egui::Ui, state: &AppState) {
    match state {
        AppState::Idle => {
            ui.label(RichText::new("選擇資料夾後即可開始。"));
        }
        AppState::Running {
            completed,
            total,
            current_file,
        } => {
            let progress = if *total == 0 {
                0.0
            } else {
                *completed as f32 / *total as f32
            };
            ui.add(
                egui::ProgressBar::new(progress)
                    .show_percentage()
                    .animate(true)
                    .desired_width(f32::INFINITY),
            );
            ui.label(format!("處理中：{current_file}"));
        }
        AppState::Finished { succeeded, failed } => {
            let color = if *failed == 0 {
                Color32::from_rgb(27, 153, 91)
            } else {
                Color32::from_rgb(219, 137, 23)
            };
            ui.label(
                RichText::new(format!("轉換完成｜成功 {succeeded} 張，失敗 {failed} 張"))
                    .color(color)
                    .strong(),
            );
        }
        AppState::Error(error) => {
            ui.label(
                RichText::new(error)
                    .color(Color32::from_rgb(220, 70, 70))
                    .strong(),
            );
        }
    }
}

fn configure_style(context: &egui::Context) {
    let theme = egui::Theme::Light;
    context.set_theme(theme);
    let mut style = (*context.style_of(theme)).clone();
    style.spacing.item_spacing = [10.0, 8.0].into();
    style.spacing.button_padding = [14.0, 8.0].into();
    style.visuals.panel_fill = Color32::from_rgb(247, 248, 250);
    style.visuals.window_corner_radius = 12.0.into();
    style.visuals.widgets.inactive.corner_radius = 7.0.into();
    style.visuals.widgets.hovered.corner_radius = 7.0.into();
    style.visuals.widgets.active.corner_radius = 7.0.into();
    context.set_style_of(theme, style);
}

fn load_chinese_font(context: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msjh.ttc",
        r"C:\Windows\Fonts\msjhbd.ttc",
        r"C:\Windows\Fonts\mingliu.ttc",
    ];
    let Some(font_bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };

    let font_name = "system-cjk".to_owned();
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        font_name.clone(),
        Arc::new(FontData::from_blob(Arc::new(font_bytes), 0)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.clone());
    }
    context.set_fonts(fonts);
}

fn run_conversion(
    input_dir: PathBuf,
    output_dir: PathBuf,
    quality: u8,
    sender: Sender<WorkerMessage>,
    repaint_context: egui::Context,
) {
    let mut files = Vec::new();
    if let Err(error) = scan_heic_files(&input_dir, &mut files) {
        send_message(
            &sender,
            WorkerMessage::Fatal(format!("掃描資料夾失敗：{error:#}")),
            &repaint_context,
        );
        return;
    }
    files.sort_unstable();
    let total = files.len();
    send_message(&sender, WorkerMessage::Scanned(total), &repaint_context);

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
                convert_heic_to_jpeg(&input_path, &output_path, quality)
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
        send_message(
            &sender,
            WorkerMessage::Progress {
                completed: index + 1,
                total,
                succeeded,
                failed,
                file_name,
                error,
            },
            &repaint_context,
        );
    }

    send_message(
        &sender,
        WorkerMessage::Finished { succeeded, failed },
        &repaint_context,
    );
}

fn send_message(
    sender: &Sender<WorkerMessage>,
    message: WorkerMessage,
    repaint_context: &egui::Context,
) {
    let _ = sender.send(message);
    repaint_context.request_repaint();
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
}
