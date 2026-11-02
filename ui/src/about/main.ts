// About window: version, licence, the no-network statement, links.
import { invoke } from "@tauri-apps/api/core";
import { t, type StringKey } from "../strings";

const REPO_URL = "https://github.com/aqueeb/snapshade";

for (const node of document.querySelectorAll<HTMLElement>("[data-s]")) {
  const key = node.dataset["s"] as StringKey | undefined;
  if (key) node.textContent = t(key);
}
document.title = t("about.title");

invoke<string>("app_version")
  .then((v) => {
    const el = document.getElementById("version");
    if (el) el.textContent = `Version ${v}`;
  })
  .catch((err: unknown) => console.error(err));

document.getElementById("github")?.addEventListener("click", () => {
  void invoke("open_url", { url: REPO_URL }).catch((err: unknown) => console.error(err));
});
document.getElementById("options")?.addEventListener("click", () => {
  void invoke("open_options").catch((err: unknown) => console.error(err));
});
