# HEIC Converter

使用純 Rust 將資料夾內的 `.heic`／`.heif` 圖片批次轉成 JPEG，並保留原本的子資料夾結構。程式提供 Windows 圖形介面、資料夾選擇、品質設定、進度與處理紀錄。

## 執行

開發模式：

```powershell
cargo run --release
```

啟動後，在視窗內選擇來源與輸出資料夾，再按下「開始轉換」。

編譯成獨立執行檔：

```powershell
cargo build --release
```

執行檔位於 `target\release\heic-converter.exe`，可以直接雙擊啟動，不會另外開啟命令列視窗。

## 相容性

目前使用 `zune-heic` 解碼器，不需要安裝 C/C++ 函式庫。它支援一般 HEIC 靜態圖片、裁切、旋轉、鏡像、ICC 與 Exif 讀取，但目前只解碼主要圖片，而且不支援 10-bit、4:2:2 或 4:4:4 HEVC 圖片。
