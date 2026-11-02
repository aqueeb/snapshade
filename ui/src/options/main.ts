// Options dialog (OPT-01..06, HOTKEY-02/03/04, SHELL-04): a plain form over the live
// `Settings`. Every change is saved immediately through `set_settings`; a rejected hotkey
// shows the validation message and reverts the recorder.
import { invoke } from "@tauri-apps/api/core";
import { comboFromEvent, displayCombo, isValidBinding, sameCombo, type KeyCombo } from "../core/hotkey";
import { t, type StringKey } from "../strings";

type ImageFormat = "Png" | { Jpeg: { quality: number } } | "Bmp";

interface Settings {
  show_notifications: boolean;
  capture_cursor: boolean;
  keep_selection_position: boolean;
  capture_hotkey: KeyCombo;
  instant_save_enabled: boolean;
  instant_save_hotkey: KeyCombo | null;
  default_format: ImageFormat;
  jpeg_quality: number;
  optimize_png: boolean;
  default_folder: string | null;
  last_save_folder: string | null;
  start_at_login: boolean;
  language: string;
  color: string;
  marker_color: string;
  tool_size: number;
}

const IS_MAC = navigator.platform.toLowerCase().includes("mac") || navigator.userAgent.includes("Macintosh");

function el<T extends HTMLElement>(id: string): T {
  const e = document.getElementById(id);
  if (!e) throw new Error(`options: #${id} missing`);
  return e as T;
}

function applyStrings(): void {
  for (const node of document.querySelectorAll<HTMLElement>("[data-s]")) {
    const key = node.dataset["s"] as StringKey | undefined;
    if (key) node.textContent = t(key);
  }
  document.title = t("options.title");
}

let settings: Settings;
let recording: "capture" | "instant" | null = null;

function formatName(f: ImageFormat): "png" | "jpeg" | "bmp" {
  if (f === "Png") return "png";
  if (f === "Bmp") return "bmp";
  return "jpeg";
}

function fill(): void {
  el<HTMLInputElement>("show_notifications").checked = settings.show_notifications;
  el<HTMLInputElement>("capture_cursor").checked = settings.capture_cursor;
  el<HTMLInputElement>("keep_selection_position").checked = settings.keep_selection_position;
  el<HTMLInputElement>("start_at_login").checked = settings.start_at_login;
  el<HTMLSelectElement>("language").value = settings.language;
  el<HTMLButtonElement>("capture_hotkey").textContent = displayCombo(settings.capture_hotkey, IS_MAC);
  el<HTMLInputElement>("instant_save_enabled").checked = settings.instant_save_enabled;
  const instant = el<HTMLButtonElement>("instant_save_hotkey");
  instant.textContent = settings.instant_save_hotkey ? displayCombo(settings.instant_save_hotkey, IS_MAC) : "—";
  instant.disabled = !settings.instant_save_enabled;
  el<HTMLSelectElement>("default_format").value = formatName(settings.default_format);
  el<HTMLInputElement>("jpeg_quality").value = String(settings.jpeg_quality);
  el<HTMLSpanElement>("jpeg_quality_value").textContent = String(settings.jpeg_quality);
  el<HTMLSelectElement>("optimize_png").value = settings.optimize_png ? "always" : "never";
  const folder = el<HTMLSpanElement>("default_folder");
  // The OS default `export::os_default_folder` uses: Desktop on macOS, Documents\\Snapshade elsewhere.
  folder.textContent = settings.default_folder ?? (IS_MAC ? "(Desktop)" : "(Documents\\Snapshade)");
  folder.title = settings.default_folder ?? "";
}

async function save(next: Settings): Promise<boolean> {
  const error = el<HTMLDivElement>("error");
  try {
    await invoke("set_settings", { settings: next });
    settings = next;
    error.textContent = "";
    fill();
    return true;
  } catch (err: unknown) {
    error.textContent = typeof err === "string" ? err : String(err);
    fill();
    return false;
  }
}

function onChange<K extends keyof Settings>(key: K, value: Settings[K]): void {
  void save({ ...settings, [key]: value });
}

function startRecording(which: "capture" | "instant", button: HTMLButtonElement): void {
  if (recording) stopRecording();
  recording = which;
  button.classList.add("recording");
  button.textContent = "…";
}

function stopRecording(): void {
  recording = null;
  el<HTMLButtonElement>("capture_hotkey").classList.remove("recording");
  el<HTMLButtonElement>("instant_save_hotkey").classList.remove("recording");
  fill();
}

function onRecordKey(e: KeyboardEvent): void {
  if (!recording) return;
  e.preventDefault();
  if (e.key === "Escape") {
    stopRecording();
    return;
  }
  const combo = comboFromEvent(e);
  if (!combo) return; // modifier only or unsupported: keep waiting
  const which = recording;
  recording = null;
  const error = el<HTMLDivElement>("error");
  if (!isValidBinding(combo)) {
    error.textContent = t("options.hotkeys.invalid");
    stopRecording();
    return;
  }
  const other = which === "capture" ? settings.instant_save_hotkey : settings.capture_hotkey;
  if (other && settings.instant_save_enabled && sameCombo(other, combo)) {
    error.textContent = t("options.hotkeys.duplicate");
    stopRecording();
    return;
  }
  const next: Settings = which === "capture" ? { ...settings, capture_hotkey: combo } : { ...settings, instant_save_hotkey: combo };
  void save(next).then(() => stopRecording());
}

function wire(): void {
  for (const tab of document.querySelectorAll<HTMLButtonElement>(".tab")) {
    tab.addEventListener("click", () => {
      for (const t2 of document.querySelectorAll<HTMLButtonElement>(".tab")) t2.classList.toggle("active", t2 === tab);
      for (const p of document.querySelectorAll<HTMLElement>(".panel")) p.hidden = p.dataset["panel"] !== tab.dataset["tab"];
    });
  }
  el<HTMLInputElement>("show_notifications").addEventListener("change", (e) => onChange("show_notifications", (e.target as HTMLInputElement).checked));
  el<HTMLInputElement>("capture_cursor").addEventListener("change", (e) => onChange("capture_cursor", (e.target as HTMLInputElement).checked));
  el<HTMLInputElement>("keep_selection_position").addEventListener("change", (e) =>
    onChange("keep_selection_position", (e.target as HTMLInputElement).checked),
  );
  el<HTMLInputElement>("start_at_login").addEventListener("change", (e) => onChange("start_at_login", (e.target as HTMLInputElement).checked));
  el<HTMLSelectElement>("language").addEventListener("change", (e) => onChange("language", (e.target as HTMLSelectElement).value));

  const capture = el<HTMLButtonElement>("capture_hotkey");
  capture.addEventListener("click", () => startRecording("capture", capture));
  const instant = el<HTMLButtonElement>("instant_save_hotkey");
  instant.addEventListener("click", () => startRecording("instant", instant));
  el<HTMLInputElement>("instant_save_enabled").addEventListener("change", (e) => {
    const enabled = (e.target as HTMLInputElement).checked;
    // Enabling without a key yet: default to ⇧⌘8 / Shift+PrintScreen-style neighbour of the main key.
    const fallback: KeyCombo = IS_MAC
      ? { code: "Digit8", mods: { command: true, control: false, shift: true, alt: false } }
      : { code: "PrintScreen", mods: { command: false, control: false, shift: true, alt: false } };
    void save({ ...settings, instant_save_enabled: enabled, instant_save_hotkey: settings.instant_save_hotkey ?? (enabled ? fallback : null) });
  });
  window.addEventListener("keydown", onRecordKey, true);

  el<HTMLSelectElement>("default_format").addEventListener("change", (e) => {
    const v = (e.target as HTMLSelectElement).value;
    const f: ImageFormat = v === "png" ? "Png" : v === "bmp" ? "Bmp" : { Jpeg: { quality: settings.jpeg_quality } };
    onChange("default_format", f);
  });
  const quality = el<HTMLInputElement>("jpeg_quality");
  quality.addEventListener("input", () => {
    el<HTMLSpanElement>("jpeg_quality_value").textContent = quality.value;
  });
  quality.addEventListener("change", () => {
    const q = Number(quality.value);
    const f: ImageFormat = formatName(settings.default_format) === "jpeg" ? { Jpeg: { quality: q } } : settings.default_format;
    void save({ ...settings, jpeg_quality: q, default_format: f });
  });
  el<HTMLSelectElement>("optimize_png").addEventListener("change", (e) => onChange("optimize_png", (e.target as HTMLSelectElement).value === "always"));
  el<HTMLButtonElement>("browse").addEventListener("click", async () => {
    const picked = await invoke<string | null>("pick_folder");
    if (picked) onChange("default_folder", picked);
  });
}

async function main(): Promise<void> {
  applyStrings();
  settings = await invoke<Settings>("get_settings");
  fill();
  wire();
}

main().catch((err: unknown) => {
  el<HTMLDivElement>("error").textContent = String(err);
});
