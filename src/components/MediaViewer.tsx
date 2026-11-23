import { useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion, type PanInfo } from "framer-motion";
import { ArrowUpRight, ChevronLeft, ChevronRight, Loader2, Pause, Play, Volume2, VolumeX, X } from "lucide-react";
import type { BannerMedia, BannerView } from "../lib/types";
import { cn } from "../lib/cn";

/*
  Окно просмотра баннера: фото и видео листаются (стрелки, клавиши, свайп),
  под ними заголовок, текст и кнопки. Видео - свой плеер: пуск, перемотка,
  время, звук. Файлы приходят уже из кэша программы (src - локальный адрес).
*/

interface Props {
  banner: BannerView | null;
  /** Адреса файлов по ключам; нет ключа - файл ещё качается. */
  srcs: Record<string, string>;
  /** Файлы, которые не удалось скачать. */
  failed?: Record<string, true>;
  /** Попробовать скачать ещё раз. */
  onRetry?: () => void;
  accent: string;
  start?: number;
  onClose: () => void;
  onOpenUrl: (url: string) => void;
}

const ease = [0.22, 1, 0.36, 1] as const;

function fmtTime(s: number): string {
  if (!Number.isFinite(s) || s < 0) return "0:00";
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

/** Видео со своими кнопками. active=false - поставить на паузу (слайд ушёл). */
function VideoPlayer({ src, poster, active }: { src: string; poster?: string; active: boolean }) {
  const v = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [t, setT] = useState(0);
  const [dur, setDur] = useState(0);
  const [started, setStarted] = useState(false);

  useEffect(() => {
    if (!active) v.current?.pause();
  }, [active]);

  const toggle = () => {
    const el = v.current;
    if (!el) return;
    if (el.paused) {
      setStarted(true);
      void el.play().catch(() => {});
    } else el.pause();
  };

  const seek = (x: number) => {
    const el = v.current;
    if (el && dur) el.currentTime = (x / 1000) * dur;
  };

  return (
    <div className="group/video relative h-full w-full bg-black">
      <video
        ref={v}
        src={src}
        poster={poster}
        playsInline
        preload="metadata"
        muted={muted}
        onClick={toggle}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
        onTimeUpdate={(e) => setT(e.currentTarget.currentTime)}
        onLoadedMetadata={(e) => setDur(e.currentTarget.duration)}
        className="h-full w-full cursor-pointer object-contain"
      />
      {/* Большая кнопка, пока видео стоит */}
      <AnimatePresence>
        {!playing && (
          <motion.button
            type="button"
            onClick={toggle}
            aria-label="Смотреть"
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 1.08 }}
            transition={{ duration: 0.18, ease }}
            className="absolute left-1/2 top-1/2 flex size-16 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-black/55 text-white ring-1 ring-white/25 backdrop-blur-md transition-colors hover:bg-black/70"
          >
            <Play className="ml-1 h-6 w-6" fill="currentColor" />
          </motion.button>
        )}
      </AnimatePresence>
      {/* Панель: видна после первого пуска, при паузе и при наведении */}
      {started && (
        <div
          className={cn(
            "absolute inset-x-0 bottom-0 flex items-center gap-3 bg-gradient-to-t from-black/80 via-black/40 to-transparent px-3 pb-2.5 pt-8 text-white transition-opacity duration-200",
            playing ? "opacity-0 group-hover/video:opacity-100 focus-within:opacity-100" : "opacity-100",
          )}
        >
          <button type="button" onClick={toggle} aria-label={playing ? "Пауза" : "Смотреть"} className="rounded-md p-1 hover:bg-white/15">
            {playing ? <Pause className="h-4 w-4" fill="currentColor" /> : <Play className="h-4 w-4" fill="currentColor" />}
          </button>
          <input
            type="range"
            min={0}
            max={1000}
            value={dur ? Math.round((t / dur) * 1000) : 0}
            onChange={(e) => seek(Number(e.target.value))}
            aria-label="Перемотка"
            className="video-range h-1 flex-1 cursor-pointer"
            style={{ ["--p" as string]: `${dur ? (t / dur) * 100 : 0}%` }}
          />
          <span className="tnum shrink-0 text-[11px] text-white/80">
            {fmtTime(t)} / {fmtTime(dur)}
          </span>
          <button type="button" onClick={() => setMuted((m) => !m)} aria-label={muted ? "Включить звук" : "Выключить звук"} className="rounded-md p-1 hover:bg-white/15">
            {muted ? <VolumeX className="h-4 w-4" /> : <Volume2 className="h-4 w-4" />}
          </button>
        </div>
      )}
    </div>
  );
}

function Slide({
  m,
  srcs,
  failed,
  onRetry,
  active,
}: {
  m: BannerMedia;
  srcs: Record<string, string>;
  failed?: Record<string, true>;
  onRetry?: () => void;
  active: boolean;
}) {
  const src = srcs[m.key];
  const poster = m.poster ? srcs[m.poster] : undefined;
  if (!src && failed?.[m.key]) {
    return (
      <div className="relative flex h-full w-full flex-col items-center justify-center gap-3 bg-black px-6 text-center">
        {poster && <img src={poster} alt="" className="absolute inset-0 h-full w-full object-contain opacity-30" />}
        <p className="relative max-w-[340px] text-[13px] leading-relaxed text-white/85">
          {m.kind === "video" ? "Видео не загрузилось." : "Фото не загрузилось."} Проверьте интернет или запустите мост и попробуйте ещё раз.
        </p>
        {onRetry && (
          <button
            type="button"
            onClick={onRetry}
            className="relative inline-flex h-8 items-center rounded-lg bg-white/15 px-3 text-[12px] font-medium text-white ring-1 ring-white/20 transition-colors hover:bg-white/25"
          >
            Попробовать ещё раз
          </button>
        )}
      </div>
    );
  }
  if (!src) {
    return (
      <div className="relative flex h-full w-full items-center justify-center bg-black">
        {poster && <img src={poster} alt="" className="absolute inset-0 h-full w-full object-contain opacity-60" />}
        <Loader2 className="relative h-6 w-6 animate-spin text-white/70" aria-label="Загружаю" />
      </div>
    );
  }
  if (m.kind === "video") return <VideoPlayer src={src} poster={poster} active={active} />;
  return <img src={src} alt="" draggable={false} className="h-full w-full select-none bg-black object-contain" />;
}

export function MediaViewer({ banner, srcs, failed, onRetry, accent, start = 0, onClose, onOpenUrl }: Props) {
  const reduce = useReducedMotion();
  const uid = useId();
  const [[i, dir], setI] = useState<[number, number]>([start, 0]);
  const panel = useRef<HTMLDivElement>(null);
  const backdropDown = useRef(false);
  const media = banner?.media ?? [];
  const n = media.length;

  useEffect(() => {
    if (banner) setI([Math.min(start, Math.max(0, n - 1)), 0]);
  }, [banner, start, n]);

  const go = useCallback((d: number) => setI(([k]) => [(k + d + n) % n, d]), [n]);

  useEffect(() => {
    if (!banner) return;
    const prev = document.activeElement as HTMLElement | null;
    panel.current?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowRight" && n > 1 && !(e.target instanceof HTMLInputElement)) go(1);
      else if (e.key === "ArrowLeft" && n > 1 && !(e.target instanceof HTMLInputElement)) go(-1);
    };
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("keydown", key);
      prev?.focus?.();
    };
  }, [banner, n, go, onClose]);

  const onDragEnd = (_: unknown, info: PanInfo) => {
    if (n < 2) return;
    if (info.offset.x < -60 || info.velocity.x < -400) go(1);
    else if (info.offset.x > 60 || info.velocity.x > 400) go(-1);
  };

  const cur = media[i];
  // Пропорции сцены - по текущему файлу, в разумных пределах.
  const ratio = cur ? Math.min(Math.max(cur.w / cur.h, 4 / 5), 21 / 9) : 16 / 9;

  return createPortal(
    <AnimatePresence>
      {banner && (
        <motion.div
          key="media-viewer"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1, transition: { duration: 0.2 } }}
          exit={{ opacity: 0, transition: { duration: 0.16 } }}
          className="fixed inset-x-0 bottom-0 top-9 z-[87] flex items-center justify-center bg-black/70 p-4 backdrop-blur-[8px] sm:p-6"
          onMouseDown={(e) => {
            backdropDown.current = e.target === e.currentTarget;
          }}
          onClick={(e) => {
            if (e.target === e.currentTarget && backdropDown.current) onClose();
            backdropDown.current = false;
          }}
        >
          <motion.div
            ref={panel}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-labelledby={`${uid}-t`}
            initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1, transition: { duration: 0.28, ease } }}
            exit={reduce ? { opacity: 0 } : { opacity: 0, y: 6, scale: 0.99, transition: { duration: 0.16 } }}
            style={{ ["--acc" as string]: accent }}
            className="relative flex max-h-full w-full max-w-[720px] flex-col overflow-hidden rounded-2xl border border-white/10 bg-vb-surface shadow-[0_30px_80px_rgba(0,0,0,0.7)] outline-none"
          >
            {/* Сцена */}
            <div className="relative w-full shrink-0 overflow-hidden bg-black" style={{ aspectRatio: String(ratio), maxHeight: "62vh" }}>
              <AnimatePresence initial={false} custom={dir} mode="popLayout">
                <motion.div
                  key={cur?.key ?? "none"}
                  custom={dir}
                  initial={reduce ? { opacity: 0 } : { x: dir >= 0 ? "100%" : "-100%", opacity: 0.4 }}
                  animate={{ x: 0, opacity: 1, transition: { duration: 0.34, ease } }}
                  exit={reduce ? { opacity: 0 } : { x: dir >= 0 ? "-30%" : "30%", opacity: 0, transition: { duration: 0.26, ease } }}
                  drag={n > 1 && cur?.kind === "image" ? "x" : false}
                  dragConstraints={{ left: 0, right: 0 }}
                  dragElastic={0.25}
                  onDragEnd={onDragEnd}
                  className="absolute inset-0"
                >
                  {cur && <Slide m={cur} srcs={srcs} failed={failed} onRetry={onRetry} active />}
                </motion.div>
              </AnimatePresence>

              {n > 1 && (
                <>
                  <button
                    type="button"
                    onClick={() => go(-1)}
                    aria-label="Назад"
                    className="absolute left-3 top-1/2 flex size-9 -translate-y-1/2 items-center justify-center rounded-full bg-black/45 text-white ring-1 ring-white/15 backdrop-blur-md transition-colors hover:bg-black/65"
                  >
                    <ChevronLeft className="h-5 w-5" />
                  </button>
                  <button
                    type="button"
                    onClick={() => go(1)}
                    aria-label="Дальше"
                    className="absolute right-3 top-1/2 flex size-9 -translate-y-1/2 items-center justify-center rounded-full bg-black/45 text-white ring-1 ring-white/15 backdrop-blur-md transition-colors hover:bg-black/65"
                  >
                    <ChevronRight className="h-5 w-5" />
                  </button>
                  <div className="pointer-events-none absolute inset-x-0 top-3 flex justify-center">
                    <div className="pointer-events-auto flex items-center gap-1.5 rounded-full bg-black/45 px-2.5 py-1.5 backdrop-blur-md">
                      {media.map((m, k) => (
                        <button
                          key={m.key + k}
                          type="button"
                          aria-label={`Слайд ${k + 1}`}
                          aria-current={k === i}
                          onClick={() => setI([k, k > i ? 1 : -1])}
                          className={cn("h-1.5 rounded-full transition-all duration-300", k === i ? "w-4 bg-white" : "w-1.5 bg-white/45 hover:bg-white/70")}
                        />
                      ))}
                    </div>
                  </div>
                </>
              )}

              <button
                type="button"
                onClick={onClose}
                aria-label="Закрыть"
                className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-full bg-black/45 text-white ring-1 ring-white/15 backdrop-blur-md transition-colors hover:bg-black/65"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* Текст и кнопки */}
            <div className="overflow-y-auto px-5 pb-5 pt-4">
              {banner.eyebrow && (
                <p className="mb-1 flex items-center gap-1.5 text-[11px] font-medium" style={{ color: "var(--acc)" }}>
                  <span aria-hidden className="h-1 w-1 rounded-full" style={{ background: "var(--acc)" }} />
                  {banner.eyebrow}
                </p>
              )}
              <h2 id={`${uid}-t`} className="text-[17px] font-semibold leading-snug tracking-[-0.01em] text-vb-fg">
                {banner.title}
              </h2>
              {banner.body && <p className="mt-1.5 whitespace-pre-line text-[13px] leading-relaxed text-vb-silver-dim">{banner.body}</p>}
              {banner.buttons.length > 0 && (
                <div className="mt-4 flex flex-wrap gap-2">
                  {banner.buttons.map((x, k) =>
                    k === 0 ? (
                      <button
                        key={x.url + k}
                        type="button"
                        onClick={() => onOpenUrl(x.url)}
                        className="inline-flex h-9 items-center gap-1 rounded-lg px-4 text-[13px] font-semibold text-black transition-[filter,transform] hover:brightness-110 active:scale-[0.97]"
                        style={{ background: "var(--acc)" }}
                      >
                        {x.label}
                        <ArrowUpRight className="h-4 w-4" aria-hidden />
                      </button>
                    ) : (
                      <button
                        key={x.url + k}
                        type="button"
                        onClick={() => onOpenUrl(x.url)}
                        className="inline-flex h-9 items-center rounded-lg border border-vb-border px-4 text-[13px] font-medium text-vb-silver transition-colors hover:bg-vb-surface-2 hover:text-vb-fg active:scale-[0.97]"
                      >
                        {x.label}
                      </button>
                    ),
                  )}
                </div>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
