import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { FolderOpen, Loader2, Search } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { appsIcons, appsSearch } from "../lib/api";
import type { FoundApp } from "../lib/types";
import { appBrand, BrandTile } from "./BrandLogos";
import { cn } from "../lib/cn";

interface Props {
  /** Пути уже добавленных приложений — их не предлагаем. */
  existing: string[];
  onAdd: (name: string, path: string) => Promise<void>;
  onClose: () => void;
}

/*
  Добавление приложения (macOS): пишете название — Spotlight ищет среди
  всех установленных программ (в т.ч. русские имена и транслит: «телег» →
  Telegram). Выбор подсказки сразу добавляет. Папка справа — ручной выбор
  .app, если программы нет в индексе. Клавиатура: ↑/↓, Enter, Esc.
*/
export function AppSearchAdd({ existing, onAdd, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<FoundApp[]>([]);
  const [searching, setSearching] = useState(false);
  const [active, setActive] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [icons, setIcons] = useState<Record<string, string>>({});
  const seq = useRef(0);

  // Настоящие иконки найденных программ (подгружаются после подсказок).
  useEffect(() => {
    const need = results.map((r) => r.path).filter((p) => !(p in icons));
    if (need.length === 0) return;
    appsIcons(need)
      .then((got) => setIcons((old) => ({ ...old, ...got })))
      .catch(() => {});
  }, [results, icons]);

  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) {
      setResults([]);
      return;
    }
    const id = ++seq.current;
    setSearching(true);
    const t = window.setTimeout(async () => {
      const found = await appsSearch(q).catch(() => [] as FoundApp[]);
      if (id !== seq.current) return; // ответ на устаревший запрос
      setResults(found.filter((f) => !existing.includes(f.path)));
      setActive(0);
      setSearching(false);
    }, 140);
    return () => window.clearTimeout(t);
  }, [query, existing]);

  const add = async (name: string, path: string) => {
    setBusy(true);
    setError(null);
    try {
      await onAdd(name, path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const pickManually = async () => {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Программа", extensions: ["app"] }],
      defaultPath: "/Applications",
    });
    if (typeof selected === "string") {
      const name = selected.replace(/\/+$/, "").split("/").pop()?.replace(/\.app$/i, "") ?? "";
      await add(name, selected);
    }
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
    if (results.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => (a + 1) % results.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => (a - 1 + results.length) % results.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const r = results[active];
      if (r) void add(r.name, r.path);
    }
  };

  const q = query.trim();
  return (
    <div className="surface-card mt-4 overflow-hidden">
      <div className="flex items-center gap-2 px-3">
        {searching || busy ? (
          <Loader2 className="h-4 w-4 shrink-0 animate-spin text-vb-silver-faint" />
        ) : (
          <Search className="h-4 w-4 shrink-0 text-vb-silver-faint" />
        )}
        <input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
          placeholder="Название программы"
          // Без автодополнения macOS: подсказка «Wec ×» над полем мешала.
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          {...{ writingsuggestions: "false" }}
          className="h-11 min-w-0 flex-1 bg-transparent text-[14px] text-vb-fg outline-none placeholder:text-vb-silver-faint"
        />
        <button
          type="button"
          onClick={pickManually}
          title="Выбрать вручную"
          aria-label="Выбрать программу вручную"
          className="rounded-md p-1.5 text-vb-silver-faint transition-colors hover:bg-vb-surface-2 hover:text-vb-silver active:scale-[0.94]"
        >
          <FolderOpen className="h-4 w-4" strokeWidth={1.9} />
        </button>
      </div>

      <AnimatePresence initial={false}>
        {(results.length > 0 || (q.length >= 2 && !searching)) && (
          <motion.ul
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden border-t border-vb-border/70 py-1"
            role="listbox"
          >
            {results.length === 0 ? (
              <li className="px-4 py-2.5 text-[12px] text-vb-silver-faint">
                Не нашлось — выберите вручную через папку справа
              </li>
            ) : (
              results.map((r, i) => {
                const brand = appBrand(r.name);
                return (
                  <li key={r.path} role="option" aria-selected={i === active}>
                    <button
                      type="button"
                      onMouseEnter={() => setActive(i)}
                      onClick={() => add(r.name, r.path)}
                      disabled={busy}
                      className={cn(
                        "flex w-full items-center gap-3 px-3 py-2 text-left transition-colors",
                        i === active ? "bg-vb-surface-2" : "hover:bg-vb-surface-2/60",
                      )}
                    >
                      {icons[r.path] ? (
                        <img src={icons[r.path]} alt="" className="h-8 w-8 shrink-0" draggable={false} />
                      ) : brand ? (
                        <BrandTile glow={brand.glow}>{brand.glyph}</BrandTile>
                      ) : (
                        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-vb-surface-2 text-[13px] font-semibold text-vb-silver">
                          {r.name.slice(0, 1).toUpperCase()}
                        </span>
                      )}
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[13px] font-medium text-vb-fg">{r.name}</span>
                        <span className="block truncate text-[11px] text-vb-silver-faint">
                          {r.path.replace(/\/[^/]+$/, "") || "/"}
                        </span>
                      </span>
                      {i === active && <span className="text-[11px] text-vb-silver-faint">↵</span>}
                    </button>
                  </li>
                );
              })
            )}
          </motion.ul>
        )}
      </AnimatePresence>
      {error && <p className="border-t border-vb-border/70 px-4 py-2 text-[12px] text-vb-loss">{error}</p>}
    </div>
  );
}
