import { useEffect, useState } from "react";

import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { useInstallInstance, useLaunchInstance } from "@/hooks/queries";

type Item = {
  id: string;
  label: string;
  disabled?: boolean;
  run: () => void;
};

type MenuState = {
  x: number;
  y: number;
  items: Item[];
};

const FIELD = "input, textarea, [contenteditable='true']";

function fieldFrom(target: EventTarget | null): HTMLElement | null {
  if (!(target instanceof HTMLElement)) return null;
  const field = target.closest(FIELD);
  return field instanceof HTMLElement ? field : null;
}

function runEdit(command: "cut" | "copy" | "paste" | "selectAll") {
  document.execCommand(command === "selectAll" ? "selectAll" : command);
}

/**
 * App-owned right-click menu.
 *
 * The stock WebView menu (Inspect, Reload, View Source) is cancelled in
 * `index.html` and again here. Text fields get Cut/Copy/Paste. Instance
 * cards and the library get launcher actions. Empty chrome gets a menu
 * with no developer entries.
 */
export function AppContextMenu() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const launch = useLaunchInstance();
  const install = useInstallInstance();
  const [menu, setMenu] = useState<MenuState | null>(null);

  useEffect(() => {
    const open = (event: MouseEvent) => {
      event.preventDefault();
      const target = event.target;
      const field = fieldFrom(target);
      const instance = target instanceof Element ? target.closest("[data-instance-id]") : null;
      const library = target instanceof Element ? target.closest("[data-context='library']") : null;
      const items: Item[] = [];

      if (field) {
        items.push(
          { id: "cut", label: t("menu.cut"), run: () => runEdit("cut") },
          { id: "copy", label: t("menu.copy"), run: () => runEdit("copy") },
          { id: "paste", label: t("menu.paste"), run: () => runEdit("paste") },
          { id: "selectAll", label: t("menu.selectAll"), run: () => runEdit("selectAll") },
        );
      } else if (instance instanceof HTMLElement) {
        const id = instance.dataset.instanceId ?? "";
        const playable = instance.dataset.instancePlayable === "1";
        items.push({
          id: playable ? "play" : "install",
          label: playable ? t("menu.play") : t("menu.install"),
          disabled: !id,
          run: () => {
            if (!id) return;
            if (playable) launch.mutate({ id });
            else install.mutate(id);
          },
        });
        items.push({
          id: "open",
          label: t("menu.open"),
          disabled: !id,
          run: () => {
            if (id) navigate(`/instances/${id}`);
          },
        });
      } else if (library) {
        items.push({
          id: "new",
          label: t("menu.newInstance"),
          run: () => window.dispatchEvent(new Event("sxml-new-instance")),
        });
      } else {
        const selected = window.getSelection()?.toString() ?? "";
        items.push({
          id: "app",
          label: t("menu.app"),
          disabled: true,
          run: () => undefined,
        });
        if (selected) {
          items.push({
            id: "copy",
            label: t("menu.copy"),
            run: () => void navigator.clipboard.writeText(selected),
          });
        }
      }

      const width = 220;
      const height = items.length * 36 + 12;
      const x = Math.min(event.clientX, window.innerWidth - width - 8);
      const y = Math.min(event.clientY, window.innerHeight - height - 8);
      setMenu({ x: Math.max(8, x), y: Math.max(8, y), items });
    };

    const close = () => setMenu(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
    };
    window.addEventListener("contextmenu", open);
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("contextmenu", open);
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", close);
    };
  }, [install, launch, navigate, t]);

  if (!menu) return null;

  return (
    <div
      role="menu"
      className="glass-strong fixed z-[80] min-w-[200px] rounded-xl border border-white/10 p-1.5 shadow-2xl"
      style={{ left: menu.x, top: menu.y }}
      onPointerDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      {menu.items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          className="flex h-8 w-full items-center rounded-lg px-3 text-left text-sm text-[var(--foreground)] hover:bg-white/8 disabled:text-[var(--muted-foreground)]"
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            item.run();
            setMenu(null);
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
