// Print page (Windows path, EXPORT-05): load the job's PNG from the print:// scheme and
// open the system print dialog; close the window when printing is done or cancelled.
import { getCurrentWindow } from "@tauri-apps/api/window";

const isWindows = navigator.userAgent.includes("Windows");
const src = isWindows ? "http://print.localhost/image.png" : "print://localhost/image.png";
const img = document.getElementById("shot") as HTMLImageElement | null;

if (img) {
  img.addEventListener("load", () => {
    setTimeout(() => window.print(), 50);
  });
  img.addEventListener("error", () => console.error("print: image failed to load"));
  img.src = `${src}?t=${Date.now()}`;
}
window.addEventListener("afterprint", () => {
  void getCurrentWindow().hide().catch((err: unknown) => console.error(err));
});
