# HEIC Converter

使用純 Rust 將資料夾內的 `.heic`／`.heif` 圖片批次轉成最高品質 JPEG，並保留原本的子資料夾結構。

## 執行

預設讀取 `input` 並輸出到 `output`：

```powershell
cargo run --release
```

也可以指定資料夾：

```powershell
cargo run --release -- "D:\Photos" "D:\Converted"
```

編譯成獨立執行檔：

```powershell
cargo build --release
```

執行檔位於 `target\release\heic-converter.exe`。

## 相容性

目前使用 `zune-heic` 解碼器，不需要安裝 C/C++ 函式庫。它支援一般 HEIC 靜態圖片、裁切、旋轉、鏡像、ICC 與 Exif 讀取，但目前只解碼主要圖片，而且不支援 10-bit、4:2:2 或 4:4:4 HEVC 圖片。
