// Screen Recording guidance (CAPTURE-03): deep link to the Privacy pane + quit-and-relaunch.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t, type StringKey } from "../strings";

const SETTINGS_URL = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

for (const node of document.querySelectorAll<HTMLElement>("[data-s]")) {
  const key = node.dataset["s"] as StringKey | undefined;
  if (key) node.textContent = t(key);
}

document.getElementById("open")?.addEventListener("click", () => {
  void invoke("open_url", { url: SETTINGS_URL }).catch((err: unknown) => console.error(err));
});
document.getElementById("relaunch")?.addEventListener("click", () => {
  void invoke("relaunch_app").catch((err: unknown) => console.error(err));
});
document.getElementById("later")?.addEventListener("click", () => {
  void getCurrentWindow().hide().catch((err: unknown) => console.error(err));
});
