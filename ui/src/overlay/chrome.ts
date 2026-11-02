// DOM chrome of the overlay: the two toolbars (spec §4, §5), the size badge, the "Select
// area" tip, the colour palette, the context menu and the one-time size hint. Pure DOM
// construction and positioning — all behaviour is wired by the controller.
import type { ToolId } from "../core/annotate/tools";
import type { PhysicalRect } from "../core/geometry";
import { layoutBadge, layoutToolbars, physicalToCss } from "../core/toolbar/layout";
import type { SessionAction } from "../core/session";
import { t } from "../strings";
import { PALETTE, TOOLBAR_GAP_PT } from "../theme/overlay";
import { ICONS, type IconName } from "./icons";

export const TOOL_ORDER: readonly ToolId[] = ["pen", "line", "arrow", "rect", "ellipse", "marker", "blur", "text"];
const TOOL_LABEL: Record<ToolId, Parameters<typeof t>[0]> = {
  pen: "tool.pen",
  line: "tool.line",
  arrow: "tool.arrow",
  rect: "tool.rect",
  ellipse: "tool.ellipse",
  marker: "tool.marker",
  blur: "tool.blur",
  text: "tool.text",
};
type ActionId = Extract<SessionAction, "print" | "copy" | "saveAs" | "close">;
const ACTION_ORDER: readonly { id: ActionId; icon: IconName; label: Parameters<typeof t>[0] }[] = [
  { id: "print", icon: "print", label: "action.print" },
  { id: "copy", icon: "copy", label: "action.copy" },
  { id: "saveAs", icon: "save", label: "action.save" },
  { id: "close", icon: "close", label: "action.close" },
];

export interface ChromeHandlers {
  onTool(tool: ToolId): void;
  onColorButton(anchor: HTMLElement): void;
  onUndo(): void;
  onAction(action: ActionId): void;
  onPalette(color: string): void;
  onCustomColor(): void;
  onContextMenu(action: "fullscreen" | "clear" | "close"): void;
}

function button(title: string, icon: string, cls: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = cls;
  b.title = title;
  b.setAttribute("aria-label", title);
  b.innerHTML = icon;
  b.tabIndex = -1;
  return b;
}

export class OverlayChrome {
  readonly tools: HTMLElement;
  readonly actions: HTMLElement;
  readonly badge: HTMLElement;
  readonly tip: HTMLElement;
  readonly palette: HTMLElement;
  readonly menu: HTMLElement;
  readonly hint: HTMLElement;
  private readonly toolButtons = new Map<ToolId, HTMLButtonElement>();
  private readonly colorSwatch: HTMLElement;
  private readonly undoButton: HTMLButtonElement;

  constructor(root: HTMLElement, h: ChromeHandlers) {
    this.tools = document.createElement("div");
    this.tools.id = "tools";
    this.tools.className = "strip vertical";
    this.tools.hidden = true;
    for (const tool of TOOL_ORDER) {
      const b = button(t(TOOL_LABEL[tool]), ICONS[tool], "tb");
      b.dataset["tool"] = tool;
      b.addEventListener("pointerdown", (e) => e.stopPropagation());
      b.addEventListener("click", () => h.onTool(tool));
      this.toolButtons.set(tool, b);
      this.tools.appendChild(b);
    }
    const color = button(t("tool.color"), "", "tb color");
    this.colorSwatch = document.createElement("span");
    this.colorSwatch.className = "swatch";
    color.appendChild(this.colorSwatch);
    color.addEventListener("pointerdown", (e) => e.stopPropagation());
    color.addEventListener("click", () => h.onColorButton(color));
    this.tools.appendChild(color);
    this.undoButton = button(t("tool.undo"), ICONS.undo, "tb");
    this.undoButton.addEventListener("pointerdown", (e) => e.stopPropagation());
    this.undoButton.addEventListener("click", () => h.onUndo());
    this.tools.appendChild(this.undoButton);

    this.actions = document.createElement("div");
    this.actions.id = "actions";
    this.actions.className = "strip horizontal";
    this.actions.hidden = true;
    for (const a of ACTION_ORDER) {
      const b = button(t(a.label), ICONS[a.icon], "tb");
      b.addEventListener("pointerdown", (e) => e.stopPropagation());
      b.addEventListener("click", () => h.onAction(a.id));
      this.actions.appendChild(b);
    }

    this.badge = document.createElement("div");
    this.badge.id = "badge";
    this.badge.hidden = true;

    this.tip = document.createElement("div");
    this.tip.id = "tip";
    this.tip.textContent = t("overlay.selectArea");
    this.tip.hidden = true;

    this.palette = document.createElement("div");
    this.palette.id = "palette";
    this.palette.hidden = true;
    this.palette.addEventListener("pointerdown", (e) => e.stopPropagation());
    for (const c of PALETTE) {
      const s = document.createElement("button");
      s.type = "button";
      s.className = "pal";
      s.style.background = c;
      s.title = c;
      s.setAttribute("aria-label", c);
      s.addEventListener("click", () => h.onPalette(c));
      this.palette.appendChild(s);
    }
    const custom = document.createElement("button");
    custom.type = "button";
    custom.className = "pal custom";
    custom.textContent = t("tool.color.custom");
    custom.addEventListener("click", () => h.onCustomColor());
    this.palette.appendChild(custom);

    this.menu = document.createElement("div");
    this.menu.id = "ctxmenu";
    this.menu.hidden = true;
    this.menu.addEventListener("pointerdown", (e) => e.stopPropagation());
    for (const [action, key] of [
      ["fullscreen", "overlay.menu.fullscreen"],
      ["clear", "overlay.menu.clear"],
      ["close", "overlay.menu.cancel"],
    ] as const) {
      const item = document.createElement("button");
      item.type = "button";
      item.className = "mi";
      item.textContent = t(key);
      item.addEventListener("click", () => h.onContextMenu(action));
      this.menu.appendChild(item);
    }

    this.hint = document.createElement("div");
    this.hint.id = "hint";
    this.hint.textContent = t("overlay.sizeHint");
    this.hint.hidden = true;

    for (const el of [this.tools, this.actions, this.badge, this.tip, this.palette, this.menu, this.hint]) root.appendChild(el);
  }

  setActiveTool(tool: ToolId | null): void {
    for (const [id, b] of this.toolButtons) b.classList.toggle("active", id === tool);
  }

  setColor(color: string): void {
    this.colorSwatch.style.background = color;
  }

  setCanUndo(can: boolean): void {
    this.undoButton.disabled = !can;
  }

  /** Position every selection-attached element for `rect` (physical px) on a `scale` panel. */
  layout(rect: PhysicalRect | null, scale: number, showToolbars: boolean): void {
    if (!rect) {
      this.tools.hidden = true;
      this.actions.hidden = true;
      this.badge.hidden = true;
      return;
    }
    const sel = physicalToCss(rect, scale);
    const bounds = { w: window.innerWidth, h: window.innerHeight };
    this.badge.hidden = false;
    this.badge.textContent = `${rect.w}x${rect.h}`;
    const badgeBox = { w: this.badge.offsetWidth || 48, h: this.badge.offsetHeight || 18 };
    const b = layoutBadge(sel, bounds, badgeBox, TOOLBAR_GAP_PT);
    this.badge.style.transform = `translate(${b.x}px, ${b.y}px)`;

    this.tools.hidden = !showToolbars;
    this.actions.hidden = !showToolbars;
    if (!showToolbars) return;
    const l = layoutToolbars({
      sel,
      bounds,
      tools: { w: this.tools.offsetWidth || 30, h: this.tools.offsetHeight || 300 },
      actions: { w: this.actions.offsetWidth || 120, h: this.actions.offsetHeight || 30 },
      gap: TOOLBAR_GAP_PT,
    });
    this.tools.style.transform = `translate(${l.tools.x}px, ${l.tools.y}px)`;
    this.actions.style.transform = `translate(${l.actions.x}px, ${l.actions.y}px)`;
  }

  showPalette(anchor: HTMLElement): void {
    const r = anchor.getBoundingClientRect();
    this.palette.hidden = false;
    const w = this.palette.offsetWidth || 160;
    const h = this.palette.offsetHeight || 120;
    const x = Math.min(Math.max(0, r.left - w - TOOLBAR_GAP_PT), window.innerWidth - w);
    const y = Math.min(Math.max(0, r.top), window.innerHeight - h);
    this.palette.style.transform = `translate(${x}px, ${y}px)`;
  }

  hidePalette(): void {
    this.palette.hidden = true;
  }

  showMenu(x: number, y: number): void {
    this.menu.hidden = false;
    const w = this.menu.offsetWidth || 140;
    const h = this.menu.offsetHeight || 90;
    this.menu.style.transform = `translate(${Math.min(x, window.innerWidth - w)}px, ${Math.min(y, window.innerHeight - h)}px)`;
  }

  hideMenu(): void {
    this.menu.hidden = true;
  }

  moveTip(x: number, y: number): void {
    this.tip.hidden = false;
    this.tip.style.transform = `translate(${x + 16}px, ${y + 20}px)`;
  }

  hideTip(): void {
    this.tip.hidden = true;
  }

  showHint(): void {
    this.hint.hidden = false;
    window.setTimeout(() => {
      this.hint.hidden = true;
    }, 4000);
  }

  /** Hide every floating element (new frame, dismissal). */
  reset(): void {
    this.layout(null, 1, false);
    this.hidePalette();
    this.hideMenu();
    this.hideTip();
    this.hint.hidden = true;
  }
}
