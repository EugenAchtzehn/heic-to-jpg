// 使用 tsx 取代 ts-node 可以不用 --loader ts-node/esm app.ts 之類的寫法
import { promisify } from "util";
import fs from "fs";
import path from "path";
import convert from "heic-convert";

const readdir = promisify(fs.readdir);
const readFile = promisify(fs.readFile);
const writeFile = promisify(fs.writeFile);
const stat = promisify(fs.stat);
const mkdir = promisify(fs.mkdir);

// 遞迴掃描資料夾，以取得所有 .heic 檔案
async function scanHeicFiles(dir: string): Promise<string[]> {
  const files: string[] = [];
  const items = await readdir(dir);

  for (const item of items) {
    const fullPath = path.join(dir, item);
    const stats = await stat(fullPath);

    if (stats.isDirectory()) {
      // 遞迴掃描子資料夾
      const subFiles = await scanHeicFiles(fullPath);
      files.push(...subFiles);
    } else if (stats.isFile() && path.extname(item).toLowerCase() === ".heic") {
      files.push(fullPath);
    }
  }

  return files;
}

// 轉換 HEIC 檔案為 JPEG
async function convertHeicToJpeg(inputPath: string, outputPath: string) {
  try {
    console.log(`正在轉換: ${inputPath}`);

    const inputBuffer = await readFile(inputPath);
    const outputBuffer = await convert({
      buffer: inputBuffer,
      format: "JPEG",
      quality: 1, // 最高品質
    });

    // 確保輸出目錄存在
    const outputDir = path.dirname(outputPath);
    await mkdir(outputDir, { recursive: true });

    await writeFile(outputPath, Buffer.from(outputBuffer));
    console.log(`轉換完成: ${outputPath}`);
  } catch (error) {
    console.error(`轉換失敗 ${inputPath}:`, error);
  }
}

(async () => {
  try {
    const inputDir = "./input";
    const outputDir = "./output";

    console.log("開始掃描 HEIC 檔案...");
    const heicFiles = await scanHeicFiles(inputDir);

    if (heicFiles.length === 0) {
      console.log("沒有找到任何 .heic 檔案");
      return;
    }

    console.log(`找到 ${heicFiles.length} 個 HEIC 檔案`);

    // 轉換每個檔案
    for (const heicFile of heicFiles) {
      // 計算相對路徑並生成對應的輸出路徑
      const relativePath = path.relative(inputDir, heicFile);
      const outputPath = path.join(outputDir, relativePath.replace(/\.heic$/i, ".jpg"));

      await convertHeicToJpeg(heicFile, outputPath);
    }

    console.log("所有檔案轉換完成！");
  } catch (error) {
    console.error("執行錯誤:", error);
  }
})();
