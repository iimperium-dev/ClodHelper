import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { ArrowUpRight, Images, Play, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { bannerMedia, noticeEvent, noticesBanners, noticesBannerDismiss } from "../lib/api";
import { useStatus } from "../lib/status";
import type { BannerView } from "../lib/types";
import { cn } from "../lib/cn";
import { MediaViewer } from "./MediaViewer";

/*
  Баннер на «Обзоре»: объявление из админки с оформлением. Что показывать,
  кому и когда - решается в админке (заголовок, текст, цвет, метка, до двух
  кнопок, можно ли закрыть, фото и видео). Программа только рисует: ссылки
  только https, фото и видео - из своего кэша, сверенные с ключом, чужих
  адресов нет. Нажатие открывает окно просмотра. Закрытый крестиком не
  возвращается.

  Условие показа тоже из админки: «всегда», «пока не работает ни один прокси
  N минут» (предложение по ситуации) или «в мастере» рядом с полем прокси.
  Решает программа на месте: на сервер про прокси ничего не уходит.
*/

/** Цвет акцента: точка, метка, свечение в углу и главная кнопка. */
const ACCENT: Record<BannerView["accent"], string> = {
  emerald: "var(--color-vb-emerald)",
  blue: "#6aa8ff",
  violet: "#a98bff",
  amber: "var(--color-vb-warn)",
  rose: "#ff7a95",
  neutral: "#d8dde3",
};

/** Подходит ли баннер месту и ситуации. */
export function bannerFits(x: BannerView, place: "overview" | "setup", downSince: number | null, now: number): boolean {
  if (place === "setup") return x.when === "setup";
  if (x.when === "always") return true;
  return x.when === "proxy_down" && downSince !== null && now - downSince >= (x.after_min ?? 0) * 60_000;
}

export function PromoBanner({ className, place = "overview" }: { className?: string; place?: "overview" | "setup" }) {
  const reduce = useReducedMotion();
  const [list, setList] = useState<BannerView[]>([]);
  const [gone, setGone] = useState<string[]>([]);
  // Статус обновляется раз в 5 с - этого хватает, чтобы баннер по ситуации
  // появился в срок и ушёл, когда прокси снова заработал.
  const { poolDownSince } = useStatus();
  const b = list.find((x) => !gone.includes(x.id) && bannerFits(x, place, poolDownSince, Date.now())) ?? null;
  const shown = useRef<string | null>(null);
  const [srcs, setSrcs] = useState<Record<string, string>>({});
  const [viewer, setViewer] = useState(false);

  useEffect(() => {
    const load = () => void noticesBanners().then((v) => setList(v ?? [])).catch(() => setList([]));
    load();
    // Рассылки обновились (админка включила или сняла баннер) - перечитать.
    const un = listen("notices:changed", load);
    return () => void un.then((f) => f());
  }, []);

  useEffect(() => {
    if (b && shown.current !== b.id) {
      shown.current = b.id;
      void noticeEvent(b.id, "shown").catch(() => {});
    }
  }, [b]);

  // Обложка - сразу, остальное - при открытии окна просмотра.
  // В мастере места мало: без обложки, фото и видео - по кнопке «Посмотреть».
  const cover = !!b?.cover && place !== "setup";
  const coverKey = cover && b?.media[0] ? (b.media[0].kind === "video" ? b.media[0].poster ?? null : b.media[0].key) : null;
  useEffect(() => {
    if (!coverKey || srcs[coverKey]) return;
    void bannerMedia([coverKey]).then((m) => setSrcs((s) => ({ ...s, ...m }))).catch(() => {});
  }, [coverKey, srcs]);

  const openViewer = () => {
    if (!b || b.media.length === 0) return;
    setViewer(true);
    const want = b.media.flatMap((m) => (m.poster ? [m.poster, m.key] : [m.key])).filter((k) => !srcs[k]);
    if (!want.length) return;
    // Сначала заставки и фото (мелкие), потом видео.
    const isVideo = (k: string) => /\.(mp4|webm)$/.test(k);
    const add = (m: Record<string, string>) => setSrcs((s) => ({ ...s, ...m }));
    const big = want.filter(isVideo);
    void bannerMedia(want.filter((k) => !isVideo(k)))
      .then(add)
      .catch(() => {})
      .finally(() => {
        if (big.length) void bannerMedia(big).then(add).catch(() => {});
      });
  };

  const open = (url: string) => {
    if (!b) return;
    void noticeEvent(b.id, "clicked").catch(() => {});
    void import("@tauri-apps/plugin-opener").then((m) => m.openUrl(url));
  };

  const dismiss = () => {
    if (!b) return;
    const id = b.id;
    setGone((g) => [...g, id]);
    void noticeEvent(id, "dismissed").catch(() => {});
    void noticesBannerDismiss(id).catch(() => {});
  };

  const acc = b ? ACCENT[b.accent] ?? ACCENT.emerald : ACCENT.emerald;
  const hasMedia = !!b && b.media.length > 0;
  const first = b?.media[0];
  const coverSrc = coverKey ? srcs[coverKey] : undefined;

  return (
    <>
    <AnimatePresence initial={false}>
      {b && (
        <motion.section
          key={b.id}
          aria-label={b.title}
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.22, 1, 0.36, 1] } }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: 4, transition: { duration: 0.18, ease: "easeIn" } }}
          style={{ ["--acc" as string]: acc }}
          className={cn(
            "relative overflow-hidden rounded-2xl border border-vb-border bg-vb-surface p-4 pr-10",
            "shadow-[0_1px_0_rgba(255,255,255,0.03)_inset,0_10px_30px_rgba(0,0,0,0.35)]",
            className,
          )}
        >
          {/* Мягкое свечение цвета акцента в углу и тонкая линия сверху. */}
          <span
            aria-hidden
            className="pointer-events-none absolute -right-16 -top-20 h-44 w-44 rounded-full opacity-[0.14] blur-3xl"
            style={{ background: "var(--acc)" }}
          />
          <span
            aria-hidden
            className="pointer-events-none absolute inset-x-6 top-0 h-px opacity-50"
            style={{ background: "linear-gradient(90deg, transparent, var(--acc), transparent)" }}
          />

          {cover && first && (
            <button
              type="button"
              onClick={openViewer}
              aria-label="Фото и видео"
              className="group/cover relative -ml-4 -mr-10 -mt-4 mb-3 block aspect-[2/1] w-[calc(100%+3.5rem)] overflow-hidden bg-black"
            >
              {coverSrc ? (
                <img
                  src={coverSrc}
                  alt=""
                  draggable={false}
                  className="h-full w-full object-cover transition-transform duration-500 ease-out group-hover/cover:scale-[1.03]"
                />
              ) : (
                <span aria-hidden className="absolute inset-0 animate-pulse bg-vb-surface-2" />
              )}
              <span aria-hidden className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-gradient-to-t from-black/45 to-transparent" />
              {first.kind === "video" && (
                <span className="absolute inset-0 flex items-center justify-center">
                  <span className="flex size-11 items-center justify-center rounded-full bg-black/55 text-white ring-1 ring-white/20 backdrop-blur-sm transition-transform group-hover/cover:scale-105">
                    <Play size={18} className="ml-0.5" fill="currentColor" />
                  </span>
                </span>
              )}
              {b.media.length > 1 && (
                <span className="absolute bottom-2 right-2 rounded-full bg-black/60 px-2 py-0.5 text-[11px] text-white backdrop-blur-sm">
                  1 / {b.media.length}
                </span>
              )}
            </button>
          )}
          {b.eyebrow && (
            <p className="relative mb-1 flex items-center gap-1.5 text-[11px] font-medium" style={{ color: "var(--acc)" }}>
              <span aria-hidden className="h-1 w-1 rounded-full" style={{ background: "var(--acc)" }} />
              {b.eyebrow}
            </p>
          )}
          <h2 className="relative text-[14px] font-semibold leading-snug tracking-[-0.01em] text-vb-fg">{b.title}</h2>
          {b.body && (
            <p className="relative mt-1 line-clamp-3 whitespace-pre-line text-[12px] leading-relaxed text-vb-silver-dim">{b.body}</p>
          )}

          {(b.buttons.length > 0 || (hasMedia && !cover)) && (
            <div className="relative mt-3 flex flex-wrap gap-2">
              {b.buttons.map((x, i) =>
                i === 0 ? (
                  <button
                    key={x.url + i}
                    type="button"
                    onClick={() => open(x.url)}
                    className="inline-flex h-8 items-center gap-1 rounded-lg px-3 text-[12px] font-semibold text-black transition-[filter,transform] hover:brightness-110 active:scale-[0.97]"
                    style={{ background: "var(--acc)" }}
                  >
                    {x.label}
                    <ArrowUpRight className="h-3.5 w-3.5" aria-hidden />
                  </button>
                ) : (
                  <button
                    key={x.url + i}
                    type="button"
                    onClick={() => open(x.url)}
                    className="inline-flex h-8 items-center rounded-lg border border-vb-border px-3 text-[12px] font-medium text-vb-silver transition-colors hover:bg-vb-surface-2 hover:text-vb-fg active:scale-[0.97]"
                  >
                    {x.label}
                  </button>
                ),
              )}
              {hasMedia && !cover && (
                <button
                  type="button"
                  onClick={openViewer}
                  className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-vb-border px-3 text-[12px] font-medium text-vb-silver transition-colors hover:bg-vb-surface-2 hover:text-vb-fg active:scale-[0.97]"
                >
                  <Images className="h-3.5 w-3.5" aria-hidden />
                  Посмотреть
                </button>
              )}
            </div>
          )}

          {b.dismissible && (
            <button
              type="button"
              onClick={dismiss}
              aria-label="Скрыть"
              title="Скрыть"
              className={cn(
                "absolute right-2.5 top-2.5 p-1 transition-colors",
                cover && first
                  ? "rounded-full bg-black/50 text-white backdrop-blur-sm hover:bg-black/70"
                  : "rounded-md text-vb-silver-faint hover:bg-vb-surface-2 hover:text-vb-silver",
              )}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </motion.section>
      )}
    </AnimatePresence>
    <MediaViewer
      banner={viewer ? b : null}
      srcs={srcs}
      accent={acc}
      onClose={() => setViewer(false)}
      onOpenUrl={open}
    />
    </>
  );
}
