import { Channel, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type ConversionEvent =
  | { type: "scanned"; total: number }
  | {
      type: "progress";
      completed: number;
      total: number;
      succeeded: number;
      failed: number;
      fileName: string;
      error: string | null;
    }
  | { type: "finished"; succeeded: number; failed: number };

interface ConversionSummary {
  succeeded: number;
  failed: number;
}

interface DefaultDirectories {
  inputDir: string;
  outputDir: string;
}

const inputDirectory = element<HTMLInputElement>("input-directory");
const outputDirectory = element<HTMLInputElement>("output-directory");
const quality = element<HTMLInputElement>("quality");
const qualityValue = element<HTMLOutputElement>("quality-value");
const browseInput = element<HTMLButtonElement>("browse-input");
const browseOutput = element<HTMLButtonElement>("browse-output");
const convertButton = element<HTMLButtonElement>("convert-button");
const clearLogButton = element<HTMLButtonElement>("clear-log");
const statusText = element<HTMLParagraphElement>("status-text");
const progressCount = element<HTMLParagraphElement>("progress-count");
const progressFill = element<HTMLDivElement>("progress-fill");
const progressTrack = document.querySelector<HTMLElement>(".progress-track");
const logList = element<HTMLDivElement>("log-list");

let running = false;

function element<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing element #${id}`);
  return value as T;
}

function setRunning(value: boolean): void {
  running = value;
  for (const control of [inputDirectory, outputDirectory, quality, browseInput, browseOutput]) {
    control.disabled = value;
  }
  convertButton.disabled = value;
  convertButton.classList.toggle("is-loading", value);
  convertButton.lastChild!.textContent = value ? " 轉換中…" : " 開始轉換";
}

function setProgress(completed: number, total: number): void {
  const percent = total === 0 ? 0 : Math.round((completed / total) * 100);
  progressFill.style.width = `${percent}%`;
  progressCount.textContent = `${completed} / ${total}`;
  progressTrack?.setAttribute("aria-valuenow", percent.toString());
}

function addLog(message: string, tone: "success" | "error" | "info" = "info"): void {
  logList.querySelector(".empty-log")?.remove();
  const row = document.createElement("div");
  row.className = `log-entry ${tone}`;

  const dot = document.createElement("span");
  dot.className = "log-dot";
  const text = document.createElement("p");
  text.textContent = message;

  row.append(dot, text);
  logList.append(row);
  logList.scrollTop = logList.scrollHeight;
}

function resetLog(): void {
  logList.replaceChildren();
  const empty = document.createElement("p");
  empty.className = "empty-log";
  empty.textContent = "尚未開始轉換。";
  logList.append(empty);
}

async function chooseDirectory(target: HTMLInputElement, title: string): Promise<void> {
  const selected = await open({
    directory: true,
    multiple: false,
    title,
    defaultPath: target.value || undefined,
  });
  if (typeof selected === "string") target.value = selected;
}

function handleEvent(event: ConversionEvent): void {
  switch (event.type) {
    case "scanned":
      setProgress(0, event.total);
      statusText.textContent = event.total === 0 ? "沒有找到可轉換的圖片。" : `找到 ${event.total} 張圖片，準備轉換。`;
      addLog(`找到 ${event.total} 張 HEIC/HEIF 圖片。`);
      break;
    case "progress":
      setProgress(event.completed, event.total);
      statusText.textContent = `正在處理 ${event.fileName}`;
      addLog(
        event.error
          ? `失敗｜${event.fileName}｜${event.error}`
          : `完成｜${event.fileName}（成功 ${event.succeeded}、失敗 ${event.failed}）`,
        event.error ? "error" : "success",
      );
      break;
    case "finished":
      statusText.textContent = event.succeeded + event.failed === 0
        ? "來源資料夾內沒有 HEIC/HEIF 圖片。"
        : `轉換完成：成功 ${event.succeeded} 張，失敗 ${event.failed} 張。`;
      addLog(`轉換完成｜成功 ${event.succeeded} 張，失敗 ${event.failed} 張。`, event.failed === 0 ? "success" : "error");
      break;
  }
}

async function startConversion(): Promise<void> {
  if (running) return;

  const inputDir = inputDirectory.value.trim();
  const outputDir = outputDirectory.value.trim();
  if (!inputDir || !outputDir) {
    statusText.textContent = "請先選擇來源與輸出資料夾。";
    addLog("缺少來源或輸出資料夾。", "error");
    return;
  }

  setRunning(true);
  resetLog();
  setProgress(0, 0);
  statusText.textContent = "正在掃描資料夾…";
  addLog("正在搜尋 HEIC/HEIF 圖片…");

  const onEvent = new Channel<ConversionEvent>();
  onEvent.onmessage = handleEvent;

  try {
    await invoke<ConversionSummary>("convert_images", {
      inputDir,
      outputDir,
      quality: Number(quality.value),
      onEvent,
    });
  } catch (error) {
    const message = String(error);
    statusText.textContent = message;
    addLog(`錯誤｜${message}`, "error");
  } finally {
    setRunning(false);
  }
}

quality.addEventListener("input", () => {
  qualityValue.value = `${quality.value}%`;
});
browseInput.addEventListener("click", () => chooseDirectory(inputDirectory, "選擇 HEIC 來源資料夾"));
browseOutput.addEventListener("click", () => chooseDirectory(outputDirectory, "選擇 JPEG 輸出資料夾"));
convertButton.addEventListener("click", startConversion);
clearLogButton.addEventListener("click", resetLog);

invoke<DefaultDirectories>("default_directories")
  .then((directories) => {
    inputDirectory.value = directories.inputDir;
    outputDirectory.value = directories.outputDir;
  })
  .catch((error) => addLog(`無法取得預設路徑：${String(error)}`, "error"));
