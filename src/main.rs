use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use jpeg_encoder::{ColorType, Encoder};
use zune_heic::HeifDecoder;

const JPEG_QUALITY: u8 = 100;

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(failed) => {
            eprintln!("完成，但有 {failed} 個檔案轉換失敗。");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("執行錯誤：{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<usize> {
    let mut args = env::args_os().skip(1);
    let input_dir = PathBuf::from(args.next().unwrap_or_else(|| "input".into()));
    let output_dir = PathBuf::from(args.next().unwrap_or_else(|| "output".into()));

    if args.next().is_some() {
        bail!("用法：heic-converter [來源資料夾] [輸出資料夾]");
    }

    if !input_dir.is_dir() {
        bail!("來源資料夾不存在或不是資料夾：{}", input_dir.display());
    }

    println!("正在搜尋 HEIC/HEIF 檔案……");
    let mut files = Vec::new();
    scan_heic_files(&input_dir, &mut files)?;
    files.sort_unstable();

    if files.is_empty() {
        println!("找不到 .heic 或 .heif 檔案。");
        return Ok(0);
    }

    println!("找到 {} 個檔案。", files.len());
    let mut failed = 0;

    for input_path in files {
        let relative_path = input_path
            .strip_prefix(&input_dir)
            .context("無法計算輸入檔案的相對路徑")?;
        let output_path = output_dir.join(relative_path).with_extension("jpg");

        println!("轉換：{}", input_path.display());
        match convert_heic_to_jpeg(&input_path, &output_path) {
            Ok(()) => println!("輸出：{}", output_path.display()),
            Err(error) => {
                failed += 1;
                eprintln!("轉換失敗 {}：{error:#}", input_path.display());
            }
        }
    }

    if failed == 0 {
        println!("全部轉換完成。");
    }

    Ok(failed)
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

fn convert_heic_to_jpeg(input_path: &Path, output_path: &Path) -> Result<()> {
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
    Encoder::new(&mut encoded, JPEG_QUALITY)
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
