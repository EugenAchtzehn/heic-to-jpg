# HEIC Converter

使用 Tauri 2、TypeScript 與 Rust 製作的離線 HEIC/HEIF 批次轉檔工具。轉換工作完全在本機執行，輸出為 JPEG，並保留來源資料夾結構。

## 開發

需要 Node.js、Rust MSVC toolchain、Microsoft C++ Build Tools 與 WebView2。

```powershell
npm install
npm run tauri dev
```

## 檢查

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

## 建置安裝檔

```powershell
npm run tauri build
```

輸出位於 `src-tauri/target/release/bundle`。

## 格式支援

目前使用純 Rust 的 `zune-heic` 解碼器，不需要額外的 C/C++ HEIC runtime。若個別檔案損壞或使用尚未支援的 HEVC 格式，程式會將該檔標示為失敗並繼續處理其他圖片。
