import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Check, ChevronDown, Eye, FolderOpen, Loader2, Pencil, Plus, RotateCw, Trash2, X } from "lucide-react";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { ProxySelect } from "./Proxies";
import {
  appsSettingsSet,
  bridgeUpdate,
  browserPolicySet,
  browserPolicyStatus,
  proxiesGet,
  proxyLabelsGet,
  sitesGet,
  sitesHosts,
  sitesBypassCandidates,
  sitesBypassFromExe,
  sitesBypassGet,
  sitesBypassSet,
  sitesNormalize,
  sitesOpenExtension,
  sitesPeek,
  sitesProxyExtensions,
  sitesScopeGet,
  sitesScopeSet,
  sitesSet,
  sitesStatus,
  systemPacSet,
  systemPacStatus,
} from "../lib/api";
import { useStatus } from "../lib/status";
import type { BrowserProxyExtensions, BypassApp, SitesScope, SeenHost, SiteEntry, SitesStatus, SystemPacStatus } from "../lib/types";
import { fadeInUp, staggerContainer } from "../lib/motion";
import { isMac } from "../lib/platform";
import { cn } from "../lib/cn";

/** Кто берёт системные настройки прокси (для подписей «Во всех браузерах»). */
const BROWSERS = isMac ? "Chrome, Safari, Edge, Firefox и другие" : "Chrome, Edge, Firefox и другие";

/** Вход сайтов моста (mirror tasks::SITES_PORT). */
const SITES_SERVER = "127.0.0.1:8887";

/** Готовые сайты: все домены, к которым они обращаются, — иначе часть
    страницы (видео, картинки, вход) шла бы с настоящего IP. */
const PRESETS: { name: string; domains: string[] }[] = [
  {
    name: "YouTube",
    domains: ["youtube.com", "youtu.be", "googlevideo.com", "ytimg.com", "ggpht.com", "youtube-nocookie.com", "youtubei.googleapis.com"],
  },
  { name: "Instagram", domains: ["instagram.com", "cdninstagram.com", "fbcdn.net", "ig.me"] },
  { name: "Facebook", domains: ["facebook.com", "fb.com", "fbcdn.net", "facebook.net", "fbsbx.com", "messenger.com"] },
  { name: "X (Twitter)", domains: ["x.com", "twitter.com", "twimg.com", "t.co"] },
  { name: "Discord", domains: ["discord.com", "discord.gg", "discordapp.com", "discordapp.net", "discord.media"] },
  { name: "Netflix", domains: ["netflix.com", "nflxvideo.net", "nflximg.net", "nflxext.com", "nflxso.net"] },
  { name: "Spotify", domains: ["spotify.com", "scdn.co", "spotifycdn.com"] },
  { name: "LinkedIn", domains: ["linkedin.com", "licdn.com"] },
];

/** Двухуровневые зоны, под которыми регистрируют домены (bbc.co.uk). */
const SECOND_LEVEL = new Set(["co.uk", "org.uk", "com.au", "com.br", "co.jp", "com.ru", "co.in", "com.tr", "com.cn", "co.kr", "com.ua", "com.mx"]);

/** Общие площадки: под ними живут тысячи чужих сервисов — добавлять всю
    зону нельзя (googleapis.com увёл бы в прокси весь Google), только адрес. */
const SHARED = new Set([
  "googleapis.com", "gstatic.com", "googleusercontent.com", "cloudfront.net", "amazonaws.com",
  "akamaized.net", "akamaihd.net", "azureedge.net", "fastly.net", "cloudflare.com", "jsdelivr.net",
  "doubleclick.net", "googlesyndication.com", "google.com", "microsoft.com", "apple.com",
]);

/** Регистрируемый домен хоста: «rr3---sn-abc.googlevideo.com» → «googlevideo.com». */
function baseDomain(host: string): string {
  if (/^[\d.]+$/.test(host) || host.includes(":")) return host; // IP
  const parts = host.split(".");
  if (parts.length <= 2) return host;
  const tail2 = parts.slice(-2).join(".");
  if (SHARED.has(tail2)) return host;
  return SECOND_LEVEL.has(tail2) ? parts.slice(-3).join(".") : tail2;
}

/** Хост попадает под домен (сам домен или поддомен) — как матч в мосте. */
function covers(domain: string, host: string): boolean {
  return host === domain || host.endsWith(`.${domain}`);
}

/** Скрытые предупреждения о расширениях-перехватчиках (на этом компьютере). */
const DISMISSED_KEY = "clod.sites.dismissedIssues";

function loadDismissed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function newId(): string {
  return `site-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

/*
  Сайты: через прокси идут только они (и AI-сервисы), остальное — напрямую.
  Работает для Chromium-браузеров в режиме «Только сайты» (вход сайтов
  моста). Список применяется мостом на лету, без перезапуска. «Подсмотреть»
  показывает, к каким ещё адресам обращается сайт, — их можно добавить.
*/
export function Sites({ onOpenApps }: { onOpenApps: () => void }) {
  const { refresh } = useStatus();
  const [sites, setSites] = useState<SiteEntry[] | null>(null);
  const [status, setStatus] = useState<SitesStatus | null>(null);
  const [pool, setPool] = useState<string[]>([]);
  const [labels, setLabels] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [addInput, setAddInput] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [peek, setPeek] = useState<string | null>(null);
  const [pac, setPac] = useState<SystemPacStatus | null>(null);
  const [advanced, setAdvanced] = useState(false);
  // Где работают сайты: false — во всех браузерах (системная автонастройка),
  // true — только в выбранных (PAC их политикой). scopeView — что показано
  // (выбор «только в выбранных» вступает в силу по «Применить»).
  const [scope, setScope] = useState<SitesScope | null>(null);
  const [scopeView, setScopeView] = useState(false);
  const [draft, setDraft] = useState<Set<string>>(new Set());
  const applyScopeState = (sc: SitesScope) => {
    setScope(sc);
    setScopeView(sc.selected);
    setDraft(new Set(sc.browsers.filter((b) => b.state === "pac").map((b) => b.domain)));
  };
  const scopePac = scope?.browsers.filter((b) => b.state === "pac").map((b) => b.domain) ?? [];
  const draftChanged =
    !!scope && (scopeView !== scope.selected || draft.size !== scopePac.length || scopePac.some((d) => !draft.has(d)));

  const chooseScope = async (sel: boolean) => {
    if (sel === scopeView) return;
    if (sel || !scope?.selected) {
      setScopeView(sel); // «Только в выбранных» — по «Применить»
      return;
    }
    // Обратно «во всех»: снять политики с браузеров и включить автонастройку.
    setBusy("scope");
    setError(null);
    try {
      applyScopeState(await sitesScopeSet(false, []));
      setPac(await systemPacStatus().catch(() => null));
      setNote("Сайты снова во всех браузерах — перезапустите браузеры, где они были выбраны");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const applyScope = async () => {
    setBusy("scope");
    setError(null);
    try {
      applyScopeState(await sitesScopeSet(true, [...draft]));
      setPac(await systemPacStatus().catch(() => null));
      setNote("Готово — перезапустите выбранные браузеры. Остальные работают без Clod Helper");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };
  const [extensions, setExtensions] = useState<BrowserProxyExtensions[]>([]);
  const [showIssues, setShowIssues] = useState(false);
  const [dismissed, setDismissed] = useState<Set<string>>(loadDismissed);

  // Предупреждение скрыто, пока набор расширений в браузере тот же: новое
  // расширение-перехватчик покажет его снова.
  const issueKey = (b: BrowserProxyExtensions) => `${b.path}|${b.extensions.map((e) => e.id).sort().join(",")}`;
  const issues = extensions.filter((b) => !dismissed.has(issueKey(b)));
  const dismissIssue = (b: BrowserProxyExtensions) => {
    const next = new Set(dismissed).add(issueKey(b));
    setDismissed(next);
    try {
      localStorage.setItem(DISMISSED_KEY, JSON.stringify([...next]));
    } catch {
      /* приватный режим — просто не запоминаем */
    }
  };

  const loadStatus = useCallback(async () => {
    const [st, p, ext, sc] = await Promise.all([
      sitesStatus().catch(() => null),
      systemPacStatus().catch(() => null),
      sitesProxyExtensions().catch(() => []),
      sitesScopeGet().catch(() => null),
    ]);
    setStatus(st);
    setPac(p);
    setExtensions(ext);
    if (sc) applyScopeState(sc);
  }, []);

  // Человек ушёл отключать расширение в браузер и вернулся — перепроверить.
  useEffect(() => {
    const onFocus = () => void sitesProxyExtensions().then(setExtensions).catch(() => {});
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  /** «Во всех браузерах»: включить/выключить системную автонастройку. */
  const togglePac = async (enable: boolean) => {
    setBusy("pac");
    setError(null);
    setNote(null);
    try {
      setPac(await systemPacSet(enable));
      setNote(enable ? "Готово — браузеры подхватят за пару секунд" : "Выключено — системные настройки прокси возвращены");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    void (async () => {
      setSites(await sitesGet().catch(() => []));
      setPool(await proxiesGet().catch(() => []));
      setLabels(await proxyLabelsGet().catch(() => ({})));
    })();
    void loadStatus();
  }, [loadStatus]);

  // Изменение видно сразу (тумблер щёлкает без ожидания ответа), при ошибке
  // — откат. Пока идёт сохранение, повторные клики игнорируются, а не гасят
  // все контролы: массовое «бледнение» на долю секунды и дёргало страницу.
  const busyRef = useRef(false);
  const save = async (next: SiteEntry[], key: string, done?: string) => {
    if (busyRef.current) return false;
    busyRef.current = true;
    const prev = sites;
    setBusy(key);
    setError(null);
    setSites(next);
    try {
      setSites(await sitesSet(next));
      if (done) setNote(done);
      return true;
    } catch (e) {
      setSites(prev);
      setError(String(e));
      return false;
    } finally {
      busyRef.current = false;
      setBusy(null);
    }
  };

  // Подсказка — всплывающая плашка: сама уходит и не двигает страницу.
  useEffect(() => {
    if (!note && !error) return;
    const t = window.setTimeout(() => {
      setNote(null);
      setError(null);
    }, error ? 8000 : 3500);
    return () => window.clearTimeout(t);
  }, [note, error]);

  const addSite = async (name: string, domains: string[]) => {
    if (!sites) return;
    const site: SiteEntry = { id: newId(), name, domains, proxy: null, enabled: true };
    if (await save([...sites, site], "add", `«${name}» — через прокси`)) {
      setAdding(false);
      setAddInput("");
      setOpen(site.id);
      // Сайт без подключённого браузера не работает — а человек, добавивший
      // сайт, ждёт, что он заработает (в 0.8.0 «ничего не происходило»).
      // Включаем «Во всех браузерах» сами, если ничто не мешает: чужой
      // прокси в Windows молча не заменяем — для него есть предупреждение.
      if (pac?.supported && !pac.enabled && !pac.foreign && status?.supported && sitesBrowsers.length === 0 && !scope?.selected) {
        await togglePac(true);
        setNote(`«${name}» — через прокси. Включил «Во всех браузерах»: сайты работают в Chrome, Edge и других`);
      }
    }
  };

  const handleAdd = async () => {
    const domain = await sitesNormalize(addInput).catch(() => null);
    if (!domain) {
      setError(`«${addInput.trim()}» не похоже на адрес сайта`);
      return;
    }
    if (sites?.some((s) => s.domains.some((d) => covers(d, domain)))) {
      setError(`${domain} уже в списке`);
      return;
    }
    await addSite(domain, [domain]);
  };

  const update = (id: string, patch: Partial<SiteEntry>, done?: string) => {
    if (!sites) return;
    void save(sites.map((s) => (s.id === id ? { ...s, ...patch } : s)), id, done);
  };

  const remove = (id: string) => {
    if (!sites) return;
    void save(sites.filter((s) => s.id !== id), id, "Удалено");
  };

  /** Перевести браузер на вход сайтов. Политику «Всегда через мост», если
      она стоит, переписываем на новый вход — иначе браузер остался бы на
      старом (весь трафик). Это одно окно прав администратора. */
  const toSitesMode = async (id: string) => {
    setBusy(`browser:${id}`);
    setError(null);
    setNote(null);
    try {
      await appsSettingsSet(id, null, false);
      const policy = await browserPolicyStatus(id).catch(() => null);
      if (policy?.configured && policy.server !== SITES_SERVER) {
        await browserPolicySet(id, true);
      }
      await loadStatus();
      setNote("Готово — перезапустите браузер через мост («Приложения» → «Перезапустить»)");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleBridgeUpdate = async () => {
    setBusy("bridge");
    setError(null);
    try {
      await bridgeUpdate();
      await refresh();
      await loadStatus();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const presetsLeft = PRESETS.filter((p) => !sites?.some((s) => s.name === p.name));
  const sitesBrowsers = status?.browsers.filter((b) => b.mode === "sites") ?? [];
  // Сайты работают, если стоит системный PAC или браузер закреплён в режиме
  // «Только сайты». Браузер «весь трафик» в счёт не идёт: он ходит через
  // мост, только когда открыт через Clod Helper.
  const notWired =
    !!status?.supported &&
    !!sites?.some((s) => s.enabled) &&
    !(pac?.enabled && pac.active) &&
    sitesBrowsers.length === 0 &&
    scopePac.length === 0;
  const peekSite = sites?.find((s) => s.id === peek) ?? null;

  return (
    <motion.div
      variants={staggerContainer}
      initial="hidden"
      animate="show"
      className="mx-auto flex w-full max-w-[520px] flex-col px-6 py-6"
    >
      <motion.header variants={fadeInUp} className="flex items-end justify-between">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-[20px] font-semibold leading-tight tracking-[-0.02em] text-vb-fg">Сайты</h1>
            {/* Предупреждения (расширения-перехватчики) — свёрнуты в кружок с числом */}
            <AnimatePresence>
              {issues.length > 0 && (
                <motion.button
                  type="button"
                  initial={{ scale: 0 }}
                  animate={{ scale: 1 }}
                  exit={{ scale: 0 }}
                  transition={{ type: "spring", bounce: 0.4, duration: 0.35 }}
                  onClick={() => setShowIssues((v) => !v)}
                  aria-label={`Предупреждения: ${issues.length}`}
                  aria-expanded={showIssues}
                  className="tnum grid h-5 min-w-5 place-items-center rounded-full bg-vb-warn px-1.5 text-[11px] font-bold text-black transition-transform hover:scale-110 active:scale-95"
                >
                  {issues.length}
                </motion.button>
              )}
            </AnimatePresence>
          </div>
          <p className="text-[12px] text-vb-silver-faint">
            Через прокси — только они и AI-сервисы, остальное напрямую
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={() => setAdding((v) => !v)} disabled={sites === null}>
          {/* Как в «Приложениях»: плюс поворачивается в крестик */}
          <Plus
            className={cn(
              "h-3.5 w-3.5 transition-transform duration-200 ease-[cubic-bezier(0.22,1,0.36,1)]",
              adding && "rotate-45",
            )}
          />
          {adding ? "Закрыть" : "Добавить"}
        </Button>
      </motion.header>

      {/* ── Старый мост: сайтов не понимает ── */}
      {status && !status.supported && (
        <motion.div
          variants={fadeInUp}
          className="mt-4 flex items-center justify-between gap-3 rounded-xl border border-vb-warn/35 bg-vb-warn/[0.05] px-4 py-3"
        >
          <p className="text-[12px] leading-snug text-vb-warn">
            Вход сайтов в мосте не запущен. Обновите мост — это пара секунд
          </p>
          <Button variant="secondary" size="sm" onClick={handleBridgeUpdate} disabled={busy !== null}>
            {busy === "bridge" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCw className="h-3.5 w-3.5" />}
            Обновить мост
          </Button>
        </motion.div>
      )}

      {/* ── Сайты есть, но ни один браузер не подключён — они не работают ── */}
      {notWired && (
        <motion.div
          variants={fadeInUp}
          className="mt-4 flex items-center justify-between gap-3 rounded-xl border border-vb-warn/35 bg-vb-warn/[0.05] px-4 py-3"
        >
          <p className="text-[12px] leading-snug text-vb-warn">
            {pac?.enabled
              ? `Сайты не работают: настройку сбросила другая программа${pac.foreign ? ` (${pac.foreign})` : ""}`
              : "Сайты пока не работают — ни один браузер не подключён"}
          </p>
          {pac?.supported ? (
            <Button variant="secondary" size="sm" onClick={() => void togglePac(true)} disabled={busy !== null}>
              {busy === "pac" && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              {pac.enabled ? "Вернуть" : "Включить"}
            </Button>
          ) : (
            <Button variant="secondary" size="sm" onClick={onOpenApps}>
              Приложения
            </Button>
          )}
        </motion.div>
      )}

      {/* ── Расширения-перехватчики: по клику на кружок у заголовка ── */}
      <AnimatePresence initial={false}>
        {showIssues && issues.length > 0 && (
          <motion.div
            key="issues"
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
      {issues.map((b) => (
        <div
          key={b.path}
          className="mt-3 rounded-xl border border-vb-warn/35 bg-vb-warn/[0.05] px-4 py-3"
        >
          <p className="text-[12px] leading-snug text-vb-warn">
            В {b.name} сайты не работают: прокси перехватывает{" "}
            {b.extensions.length === 1 ? "расширение" : "расширения"}{" "}
            {b.extensions.map((e) => `«${e.name}»`).join(", ")}. Отключите{" "}
            {b.extensions.length === 1 ? "его" : "их"} или выберите в{" "}
            {b.extensions.length === 1 ? "нём" : "них"} режим «Выключено»
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            {b.extensions.map((e) => (
              <Button
                key={e.id}
                variant="secondary"
                size="sm"
                disabled={busy !== null}
                onClick={() =>
                  void (async () => {
                    setError(null);
                    try {
                      await sitesOpenExtension(b.path, e.id);
                      setNote(`Открыл «${e.name}» в ${b.name} — отключите его и вернитесь сюда`);
                    } catch (err) {
                      setError(String(err));
                    }
                  })()
                }
              >
                Открыть «{e.name}»
              </Button>
            ))}
            <Button variant="ghost" size="sm" onClick={() => dismissIssue(b)}>
              Скрыть
            </Button>
          </div>
        </div>
      ))}
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Добавление ── */}
      <AnimatePresence>
        {adding && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div className="surface-card mt-4 p-4">
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  if (addInput.trim()) void handleAdd();
                }}
                className="flex gap-2"
              >
                <input
                  type="text"
                  value={addInput}
                  onChange={(e) => {
                    setAddInput(e.target.value);
                    setError(null);
                  }}
                  placeholder="Адрес или ссылка: example.com"
                  className="min-w-0 flex-1 rounded-lg border border-vb-border bg-vb-surface-2 px-3 py-2 font-mono text-[13px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
                  autoFocus
                />
                <Button type="submit" size="sm" disabled={!addInput.trim() || busy !== null}>
                  {busy === "add" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Добавить"}
                </Button>
              </form>
              {presetsLeft.length > 0 && (
                <>
                  <div className="mb-1.5 mt-3 text-[11px] text-vb-silver-faint">Готовые — со всеми адресами сайта</div>
                  <div className="flex flex-wrap gap-1.5">
                    {presetsLeft.map((p) => (
                      <button
                        key={p.name}
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void addSite(p.name, p.domains)}
                        className="rounded-full border border-vb-border px-2.5 py-1 text-[12px] text-vb-silver-dim transition-colors hover:border-vb-emerald/40 hover:text-vb-silver disabled:opacity-40"
                      >
                        {p.name}
                      </button>
                    ))}
                  </div>
                </>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {createPortal(
        <AnimatePresence>
          {(error || note) && (
            <motion.div
              key={error ?? note}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 6 }}
              transition={{ duration: 0.18 }}
              role="status"
              className={cn(
                "pointer-events-none fixed bottom-5 left-1/2 z-[60] max-w-[420px] -translate-x-1/2 rounded-xl border px-4 py-2.5 text-center text-[12px] leading-snug shadow-[0_12px_32px_rgba(0,0,0,0.5)]",
                error ? "border-vb-loss/40 bg-vb-surface text-vb-loss" : "border-vb-border bg-vb-surface text-vb-silver",
              )}
            >
              {error ?? note}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}

      {/* ── Список сайтов ── */}
      <motion.section variants={fadeInUp} className="mt-4">
        {sites === null ? (
          <div className="surface-card flex justify-center p-6">
            <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" />
          </div>
        ) : sites.length === 0 ? (
          <div className="surface-card p-5 text-center">
            <div className="text-[13px] text-vb-silver">Сайтов пока нет</div>
            <div className="mt-1 text-[12px] text-vb-silver-faint">
              Добавьте сайт, который должен открываться через прокси, — например, YouTube
            </div>
            {!adding && (
              <Button variant="secondary" size="sm" className="mt-3" onClick={() => setAdding(true)}>
                <Plus className="h-3.5 w-3.5" />
                Добавить сайт
              </Button>
            )}
          </div>
        ) : (
          <div className="surface-card divide-y divide-vb-border/70">
            {sites.map((site) => (
              <SiteRow
                key={site.id}
                site={site}
                pool={pool}
                labels={labels}
                busy={busy}
                open={open === site.id}
                onToggleOpen={() => setOpen((v) => (v === site.id ? null : site.id))}
                onUpdate={(patch, done) => update(site.id, patch, done)}
                onRemove={() => remove(site.id)}
                onPeek={() => setPeek(site.id)}
              />
            ))}
          </div>
        )}
      </motion.section>

      {/* ── Расширенные настройки: «Во всех браузерах» (системный PAC: Windows и macOS) ── */}
      {pac?.supported && (
        <motion.section variants={fadeInUp} className="mt-6">
          <button
            type="button"
            onClick={() => setAdvanced((v) => !v)}
            className="flex items-center gap-1.5 px-1 text-[12px] text-vb-silver-faint transition-colors hover:text-vb-silver"
          >
            <ChevronDown className={cn("h-3.5 w-3.5 transition-transform duration-200", advanced && "rotate-180")} />
            Расширенные настройки
          </button>
          <AnimatePresence initial={false}>
            {advanced && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
                className="overflow-hidden"
              >
          {/* Где работают сайты: во всех браузерах или только в выбранных */}
          <div className="mt-2 flex rounded-lg bg-vb-surface-2/60 p-0.5" role="tablist" aria-label="Где работают сайты">
            {([false, true] as const).map((sel) => (
              <button
                key={String(sel)}
                type="button"
                role="tab"
                aria-selected={scopeView === sel}
                onClick={() => void chooseScope(sel)}
                disabled={busy !== null}
                className={cn(
                  "flex-1 rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors",
                  scopeView === sel ? "bg-vb-surface text-vb-fg shadow-[0_1px_2px_rgba(0,0,0,0.3)]" : "text-vb-silver-dim hover:text-vb-silver",
                )}
              >
                {sel ? "Только в выбранных" : "Во всех браузерах"}
              </button>
            ))}
          </div>

          {!scopeView ? (
            <>
          <div className="surface-card mt-2 divide-y divide-vb-border/70">
            <div className="flex items-center justify-between gap-4 px-4 py-3">
              <div className="min-w-0">
                <div className="text-[14px] font-semibold text-vb-fg">Во всех браузерах</div>
                <div
                  className={cn(
                    "mt-px text-[11px] leading-snug",
                    !status?.supported
                      ? "text-vb-silver-faint"
                      : pac.enabled && pac.active
                        ? "text-vb-emerald"
                        : pac.enabled
                          ? "text-vb-warn"
                          : "text-vb-silver-faint",
                  )}
                >
                  {!status?.supported
                    ? "Сначала обновите мост — кнопка выше"
                    : pac.enabled && pac.active
                      ? `${BROWSERS}: сайты из списка — через прокси`
                      : pac.enabled
                        ? `Настройку сбросила другая программа${pac.foreign ? ` (${pac.foreign})` : ""}`
                        : `${BROWSERS} — без настройки каждого браузера`}
                </div>
              </div>
              {pac.enabled && !pac.active ? (
                <Button variant="secondary" size="sm" disabled={busy !== null} onClick={() => void togglePac(true)}>
                  {busy === "pac" && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  Вернуть
                </Button>
              ) : (
                <Toggle
                  checked={pac.enabled}
                  disabled={busy !== null || (!pac.enabled && !status?.supported)}
                  onChange={() => void togglePac(!pac.enabled)}
                  label="Во всех браузерах"
                />
              )}
            </div>
            {!pac.enabled && pac.foreign && (
              <p className="px-4 py-2.5 text-[11px] leading-snug text-vb-warn">
                В {isMac ? "macOS" : "Windows"} уже задан {pac.foreign}. Включение заменит его, выключение — вернёт как было
              </p>
            )}
          </div>
          <p className="mt-2 px-1 text-[11px] leading-snug text-vb-silver-faint">
            Каждый браузер перед каждым запросом спрашивает, идти ли через прокси, — это несколько
            миллисекунд на запрос. Не нужно в каждом браузере — выберите «Только в выбранных»
          </p>
          <BypassCard />
            </>
          ) : (
            <>
              <div className="surface-card mt-2 divide-y divide-vb-border/70">
                {scope === null ? (
                  <div className="flex justify-center p-4">
                    <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" />
                  </div>
                ) : scope.browsers.length === 0 ? (
                  <p className="px-4 py-3 text-[12px] text-vb-silver-faint">Не нашёл Chrome, Edge, Brave или Chromium</p>
                ) : (
                  scope.browsers.map((b) => {
                    const locked = b.state === "bridge" || b.state === "conflict";
                    const on = draft.has(b.domain);
                    return (
                      <button
                        key={b.domain}
                        type="button"
                        disabled={locked || busy !== null}
                        onClick={() =>
                          setDraft((prev) => {
                            const next = new Set(prev);
                            if (next.has(b.domain)) next.delete(b.domain);
                            else next.add(b.domain);
                            return next;
                          })
                        }
                        className={cn(
                          "flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors",
                          locked ? "cursor-default opacity-60" : "hover:bg-vb-surface-2/50",
                        )}
                      >
                        <span
                          className={cn(
                            "flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors",
                            on ? "border-vb-emerald bg-vb-emerald" : "border-vb-border-strong",
                          )}
                        >
                          {on && <Check className="h-3 w-3 text-black" />}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block text-[13px] font-medium text-vb-fg">{b.name}</span>
                          <span className={cn("block truncate text-[11px]", b.state === "conflict" ? "text-vb-warn" : "text-vb-silver-faint")}>
                            {b.state === "pac"
                              ? "Сайты из списка — через прокси"
                              : b.state === "bridge"
                                ? "Весь трафик через мост — настроено в «Приложениях»"
                                : b.state === "conflict"
                                  ? b.conflict ?? "Чужая политика прокси"
                                  : "Без Clod Helper"}
                          </span>
                        </span>
                      </button>
                    );
                  })
                )}
              </div>
              <div className="mt-2 flex items-start justify-between gap-3 px-1">
                <p className="text-[11px] leading-snug text-vb-silver-faint">
                  Остальные браузеры, включая 1browser, работают совсем без Clod Helper. После изменений
                  перезапустите выбранные браузеры. Новые сайты в них могут заработать только после перезапуска
                </p>
                {scope && draftChanged && (
                  <Button size="sm" disabled={busy !== null} onClick={() => void applyScope()}>
                    {busy === "scope" && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                    Применить
                  </Button>
                )}
              </div>
            </>
          )}

              </motion.div>
            )}
          </AnimatePresence>
        </motion.section>
      )}

      {/* ── Браузер (macOS: системной автонастройки пока нет — закрепление политикой) ── */}
      {pac && !pac.supported && (
      <motion.section variants={fadeInUp} className="mt-6">
        <div className="mb-2 px-1">
          <h2 className="text-[15px] font-semibold text-vb-fg">Браузер</h2>
          <p className="text-[12px] text-vb-silver-faint">
            Сайты работают в Chrome, Edge, Brave и Chromium в режиме «Только сайты»
          </p>
        </div>
        <div className="surface-card divide-y divide-vb-border/70">
          {status === null ? (
            <div className="flex justify-center p-4">
              <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" />
            </div>
          ) : status.browsers.length === 0 ? (
            <div className="flex items-center justify-between gap-3 px-4 py-3">
              <span className="text-[12px] leading-snug text-vb-silver-faint">
                Добавьте браузер в «Приложения» — через него и пойдут сайты
              </span>
              <Button variant="secondary" size="sm" onClick={onOpenApps}>
                Приложения
              </Button>
            </div>
          ) : (
            status.browsers.map((b) => (
              <div key={b.id} className="flex items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[14px] font-semibold text-vb-fg">{b.name}</div>
                  <div
                    className={cn(
                      "mt-px text-[11px]",
                      b.mode === "sites" ? "text-vb-emerald" : "text-vb-silver-faint",
                    )}
                  >
                    {b.mode === "sites"
                      ? "Только сайты: AI и ваши сайты — через прокси"
                      : b.mode === "all"
                        ? "Весь трафик через прокси — список сайтов не нужен"
                        : "Только AI-сайты — обновите мост"}
                  </div>
                </div>
                {b.mode === "all" && status.supported && (
                  <Button
                    variant="secondary"
                    size="sm"
                    // Без живого входа сайтов браузер остался бы без сети.
                    disabled={busy !== null || !status?.supported}
                    onClick={() => void toSitesMode(b.id)}
                  >
                    {busy === `browser:${b.id}` && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                    Только сайты
                  </Button>
                )}
              </div>
            ))
          )}
        </div>
        <p className="mt-2 px-1 text-[11px] leading-snug text-vb-silver-faint">
          Открывайте браузер через «Приложения» или включите в его настройках «Всегда через мост» — тогда
          сайты пойдут через прокси, даже если браузер открыт из Dock
        </p>
      </motion.section>
      )}

      {peekSite && (
        <PeekDialog
          site={peekSite}
          sites={sites ?? []}
          browserReady={(!!pac?.enabled && pac.active) || sitesBrowsers.length > 0 || scopePac.length > 0}
          pacMode={!!pac?.enabled && pac.active}
          limited={scopePac.length > 0 && !(pac?.enabled && pac.active)}
          onAdd={async (domains) => {
            const next = [...new Set([...peekSite.domains, ...domains])];
            return save(
              (sites ?? []).map((s) => (s.id === peekSite.id ? { ...s, domains: next } : s)),
              peekSite.id,
              `Добавлено к «${peekSite.name}»: ${domains.length}`,
            );
          }}
          onClose={() => setPeek(null)}
        />
      )}
    </motion.div>
  );
}

/*
  «Не трогать эти браузеры»: программы, трафик которых Clod Helper не
  трогает вовсе — мост узнаёт их по соединению и отпускает напрямую, с
  настоящего IP. Правило — на папку программы: у антидетект-браузера
  (1browser и т.п.) сразу на все профили, сколько их ни открой.
*/
function BypassCard() {
  const [apps, setApps] = useState<BypassApp[] | null>(null);
  const [candidates, setCandidates] = useState<BypassApp[] | null>(null);
  const [picking, setPicking] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    void sitesBypassGet().then(setApps).catch(() => setApps([]));
  }, []);

  const save = async (next: BypassApp[]) => {
    const prev = apps;
    setApps(next);
    setErr(null);
    try {
      setApps(await sitesBypassSet(next));
    } catch (e) {
      setApps(prev);
      setErr(String(e));
    }
  };

  const openPicker = async () => {
    setPicking((v) => !v);
    setCandidates(null);
    setCandidates(await sitesBypassCandidates().catch(() => []));
  };

  const add = (a: BypassApp) => {
    setPicking(false);
    if (apps?.some((x) => x.dir.toLowerCase() === a.dir.toLowerCase())) return;
    void save([...(apps ?? []), a]);
  };

  const pickExe = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Программа", extensions: ["exe"] }] });
    if (typeof path !== "string") return;
    try {
      add(await sitesBypassFromExe(path));
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="surface-card mt-3">
      <div className="flex items-start justify-between gap-3 px-4 pt-3">
        <div className="min-w-0">
          <div className="text-[14px] font-semibold text-vb-fg">Не трогать эти браузеры</div>
          <div className="mt-px text-[11px] leading-snug text-vb-silver-faint">
            Их трафик идёт напрямую с вашего IP, как без Clod Helper. У антидетект-браузеров — сразу все профили
          </div>
        </div>
        <Button variant="secondary" size="sm" onClick={() => void openPicker()}>
          <Plus className={cn("h-3.5 w-3.5 transition-transform duration-200", picking && "rotate-45")} />
          {picking ? "Закрыть" : "Добавить"}
        </Button>
      </div>

      <AnimatePresence initial={false}>
        {picking && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div className="mx-4 mt-3 overflow-hidden rounded-lg border border-vb-border/70 bg-vb-surface-2/40">
              {candidates === null ? (
                <div className="flex justify-center p-3">
                  <Loader2 className="h-4 w-4 animate-spin text-vb-silver-dim" />
                </div>
              ) : (
                candidates.map((c) => (
                  <button
                    key={c.dir}
                    type="button"
                    onClick={() => add(c)}
                    className="flex w-full items-center gap-2.5 border-b border-vb-border/50 px-3 py-2 text-left transition-colors hover:bg-vb-surface-2/80"
                  >
                    <span
                      className={cn("h-1.5 w-1.5 shrink-0 rounded-full", c.running ? "bg-vb-emerald" : "bg-vb-border-strong")}
                      title={c.running ? "Запущен" : "Не запущен"}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] text-vb-fg">{c.name}</span>
                      <span className="block truncate font-mono text-[10px] text-vb-silver-faint">{c.dir}</span>
                    </span>
                    <Plus className="h-3.5 w-3.5 shrink-0 text-vb-silver-faint" />
                  </button>
                ))
              )}
              <button
                type="button"
                onClick={() => void pickExe()}
                className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-[12px] text-vb-silver-dim transition-colors hover:bg-vb-surface-2/80 hover:text-vb-silver"
              >
                <FolderOpen className="h-3.5 w-3.5" />
                Выбрать программу…
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <div className="px-4 pb-3 pt-2">
        {apps === null ? null : apps.length === 0 ? (
          <p className="text-[12px] text-vb-silver-faint">Пока сайты из списка идут через прокси во всех браузерах</p>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {apps.map((a) => (
              <span
                key={a.dir}
                title={a.dir}
                className="inline-flex items-center gap-1.5 rounded-full border border-vb-border bg-vb-surface-2/60 py-1 pl-3 pr-1 text-[12px] text-vb-silver"
              >
                {a.name}
                <button
                  type="button"
                  aria-label={`Вернуть ${a.name}`}
                  onClick={() => void save(apps.filter((x) => x.dir !== a.dir))}
                  className="rounded-full p-0.5 text-vb-silver-faint transition-colors hover:text-vb-loss"
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
        )}
        {err && <p className="mt-2 text-[11px] text-vb-loss">{err}</p>}
      </div>
    </div>
  );
}

/* ── Строка сайта: тумблер, прокси; по клику — адреса и действия ── */
function SiteRow({
  site,
  pool,
  labels,
  busy,
  open,
  onToggleOpen,
  onUpdate,
  onRemove,
  onPeek,
}: {
  site: SiteEntry;
  pool: string[];
  labels: Record<string, string>;
  busy: string | null;
  open: boolean;
  onToggleOpen: () => void;
  onUpdate: (patch: Partial<SiteEntry>, done?: string) => void;
  onRemove: () => void;
  onPeek: () => void;
}) {
  const [domainInput, setDomainInput] = useState("");
  const [domainError, setDomainError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [editing, setEditing] = useState(false);
  const [nameInput, setNameInput] = useState(site.name);
  const rowBusy = busy === site.id;

  const commitName = () => {
    setEditing(false);
    const name = nameInput.trim();
    if (name && name !== site.name) onUpdate({ name }, "Переименовано");
  };

  const addDomain = async () => {
    const d = await sitesNormalize(domainInput).catch(() => null);
    if (!d) {
      setDomainError("Не похоже на адрес сайта");
      return;
    }
    setDomainInput("");
    setDomainError(null);
    if (!site.domains.includes(d)) onUpdate({ domains: [...site.domains, d] });
  };

  return (
    <div className={cn(!site.enabled && "opacity-60")}>
      <div className="flex items-center gap-3 px-4 py-3">
        <Toggle
          checked={site.enabled}
          onChange={() => onUpdate({ enabled: !site.enabled }, site.enabled ? "Выключено — сайт идёт напрямую" : "Включено")}
          label={`${site.name} через прокси`}
        />
        <div className="group/name min-w-0 flex-1">
          {editing ? (
            <input
              autoFocus
              value={nameInput}
              maxLength={60}
              onChange={(e) => setNameInput(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitName();
                if (e.key === "Escape") setEditing(false);
              }}
              aria-label="Название сайта"
              className="w-full rounded-md border border-vb-emerald/60 bg-vb-surface-2 px-1.5 py-px text-[14px] font-semibold text-vb-fg outline-none"
            />
          ) : (
            <div className="flex items-center gap-1">
              <button type="button" onClick={onToggleOpen} className="flex min-w-0 items-center gap-1.5 text-left">
                <span className="truncate text-[14px] font-semibold text-vb-fg">{site.name}</span>
                <ChevronDown
                  className={cn("h-3.5 w-3.5 shrink-0 text-vb-silver-faint transition-transform duration-200", open && "rotate-180")}
                />
              </button>
              <button
                type="button"
                aria-label={`Переименовать ${site.name}`}
                onClick={() => {
                  setNameInput(site.name);
                  setEditing(true);
                }}
                className="shrink-0 rounded p-1 text-vb-silver-faint opacity-0 transition-opacity duration-150 hover:text-vb-silver focus-visible:opacity-100 group-hover/name:opacity-100"
              >
                <Pencil className="h-3 w-3" />
              </button>
            </div>
          )}
          <button
            type="button"
            onClick={onToggleOpen}
            className="mt-px block w-full truncate text-left font-mono text-[11px] text-vb-silver-faint"
          >
            {site.domains[0]}
            {site.domains.length > 1 && ` +${site.domains.length - 1}`}
          </button>
        </div>
        {/* Место под индикатор занято всегда — появление не сдвигает строку */}
        <Loader2 className={cn("h-4 w-4 shrink-0 animate-spin text-vb-silver-dim", !rowBusy && "invisible")} />
        <ProxySelect
          value={site.proxy ?? -1}
          pool={pool}
          labels={labels}
          disabled={pool.length === 0}
          onChange={(v) => onUpdate({ proxy: v < 0 ? null : v }, "Сохранено")}
        />
      </div>

      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="overflow-hidden"
          >
            <div className="px-4 pb-3">
              <div className="flex flex-wrap gap-1.5">
                {site.domains.map((d) => (
                  <span
                    key={d}
                    className="inline-flex items-center gap-1 rounded-full border border-vb-border bg-vb-surface-2/60 py-0.5 pl-2.5 pr-1 font-mono text-[11px] text-vb-silver"
                  >
                    {d}
                    {site.domains.length > 1 && (
                      <button
                        type="button"
                        aria-label={`Убрать ${d}`}
                        disabled={busy !== null}
                        onClick={() => onUpdate({ domains: site.domains.filter((x) => x !== d) })}
                        className="rounded-full p-0.5 text-vb-silver-faint transition-colors hover:text-vb-loss"
                      >
                        <X className="h-3 w-3" />
                      </button>
                    )}
                  </span>
                ))}
              </div>
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  if (domainInput.trim()) void addDomain();
                }}
                className="mt-2 flex gap-2"
              >
                <input
                  type="text"
                  value={domainInput}
                  onChange={(e) => {
                    setDomainInput(e.target.value);
                    setDomainError(null);
                  }}
                  placeholder="Ещё адрес: cdn.example.com"
                  className="min-w-0 flex-1 rounded-lg border border-vb-border bg-vb-surface-2 px-2.5 py-1.5 font-mono text-[12px] text-vb-fg outline-none transition-colors focus:border-vb-emerald/60"
                />
                <Button type="submit" variant="secondary" size="sm" disabled={!domainInput.trim() || busy !== null}>
                  <Plus className="h-3.5 w-3.5" />
                </Button>
              </form>
              {domainError && <p className="mt-1.5 text-[11px] text-vb-loss">{domainError}</p>}
              <p className="mt-1.5 text-[11px] text-vb-silver-faint">Поддомены входят сами: youtube.com покрывает m.youtube.com</p>
              <p className="mt-1 text-[11px] text-vb-silver-faint">
                Сайт пишет «blocked» или не открывается? Выберите справа другой прокси — сайты иногда блокируют отдельные IP
              </p>
              <div className="mt-3 flex items-center justify-between gap-2">
                <Button variant="secondary" size="sm" onClick={onPeek} disabled={!site.enabled}>
                  <Eye className="h-3.5 w-3.5" />
                  Подсмотреть адреса
                </Button>
                {confirmDelete ? (
                  <span className="flex items-center gap-2 text-[12px] text-vb-silver-dim">
                    Удалить?
                    <button
                      type="button"
                      onClick={onRemove}
                      disabled={busy !== null}
                      className="rounded-md bg-vb-loss px-2 py-0.5 text-[11px] font-semibold text-white transition-colors hover:bg-vb-loss/85 active:scale-[0.96] disabled:opacity-40"
                    >
                      Да
                    </button>
                    <button type="button" onClick={() => setConfirmDelete(false)} className="text-[11px] text-vb-silver-faint hover:text-vb-silver">
                      Нет
                    </button>
                  </span>
                ) : (
                  <Button variant="ghost" size="sm" onClick={() => setConfirmDelete(true)}>
                    <Trash2 className="h-3.5 w-3.5" />
                    Удалить
                  </Button>
                )}
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ── «Подсмотреть»: какие адреса сайт открывает, пока идёт запись ── */
interface HostGroup {
  domain: string;
  hosts: SeenHost[];
  count: number;
  /** Все адреса группы уже идут через прокси. */
  proxied: boolean;
  /** Группа уже покрыта этим сайтом. */
  inSite: boolean;
  /** Группа покрыта другим сайтом из списка. */
  otherSite: string | null;
  /** Похоже, нужна этому сайту: имя сайта в адресе или пришла вместе с его страницей. */
  related: boolean;
}

/** Реклама, аналитика и службы ОС: сайту для работы не нужны — всегда в «Прочее». */
const NOISE = [
  "doubleclick.net", "google-analytics.com", "googletagmanager.com", "googlesyndication.com",
  "googleadservices.com", "adservice.google.com", "mc.yandex.ru", "analytics.tiktok.com", "hotjar.com",
  "clarity.ms", "windowsupdate.com", "microsoft.com", "msftconnecttest.com", "live.com", "msn.com",
  "bing.com", "windows.com", "office.net", "skype.com", "gvt1.com", "mozilla.net", "mozilla.com",
];

/** Опознавательное слово сайта: «beatstars.com» → «beatstars» (короче 4 букв — не годится). */
function siteToken(site: SiteEntry): string | null {
  const t = baseDomain(site.domains[0] ?? "").split(".")[0] ?? "";
  return t.length >= 4 ? t : null;
}

/** Хосты, появившиеся раньше страницы сайта больше чем на это, — фон (другие вкладки). */
const BEFORE_PAGE_MS = 1500;

function PeekDialog({
  site,
  sites,
  browserReady,
  pacMode,
  limited,
  onAdd,
  onClose,
}: {
  site: SiteEntry;
  sites: SiteEntry[];
  browserReady: boolean;
  /** Сайты идут через системный PAC: на время записи весь трафик — через мост. */
  pacMode: boolean;
  /** «Только в выбранных»: PAC браузера постоянный — видны только адреса из списка. */
  limited?: boolean;
  onAdd: (domains: string[]) => Promise<boolean>;
  onClose: () => void;
}) {
  const [mark, setMark] = useState<number | null>(null);
  const [hosts, setHosts] = useState<SeenHost[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [adding, setAdding] = useState(false);
  const [showOther, setShowOther] = useState(false);
  // Запись идёт до 3 минут (system_pac::PEEK_LIMIT): дольше весь браузер
  // тянулся бы через мост и тормозил.
  const [expired, setExpired] = useState(false);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && closeRef.current();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const start = async () => {
    setErr(null);
    setHosts([]);
    setPicked(new Set());
    try {
      // С системным PAC адреса не из списка идут мимо моста — на время
      // записи PAC ведёт через мост всё (решает мост: чужое — напрямую).
      if (pacMode) await sitesPeek(true);
      // С чистого листа (reset): first_ms — когда адрес появился после
      // старта записи. since в будущем — пустой список, но с часами моста.
      const r = await sitesHosts(Number.MAX_SAFE_INTEGER, true);
      setShowOther(false);
      setExpired(false);
      setMark(r.now_ms);
    } catch (e) {
      setErr(String(e));
    }
  };

  // Закрыли окно — запись кончилась, PAC обратно в обычный режим.
  useEffect(() => {
    if (mark === null || !pacMode) return;
    return () => void sitesPeek(false).catch(() => {});
  }, [mark, pacMode]);

  // Через 3 минуты запись кончается сама (её выключает и программа).
  useEffect(() => {
    if (mark === null) return;
    const t = window.setTimeout(() => {
      setExpired(true);
      if (pacMode) void sitesPeek(false).catch(() => {});
    }, 180_000);
    return () => window.clearTimeout(t);
  }, [mark, pacMode]);

  useEffect(() => {
    if (mark === null) return;
    let alive = true;
    const tick = async () => {
      try {
        const r = await sitesHosts(mark);
        if (alive) {
          setHosts(r.hosts);
          setErr(null);
        }
      } catch (e) {
        if (alive) setErr(String(e));
      }
    };
    void tick();
    const t = window.setInterval(() => void tick(), 1500);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, [mark]);

  // Когда браузер начал грузить страницу сайта: первое появление его адреса.
  const pageAt = useMemo(() => {
    const own = hosts.filter((h) => site.domains.some((d) => covers(d, h.host))).map((h) => h.first_ms);
    return own.length ? Math.min(...own) : null;
  }, [hosts, site]);

  /*
    Через мост во время записи идёт трафик ВСЕХ вкладок и программ. Нужные
    сайту — те, что с его именем в адресе или появились вместе с его
    страницей (не раньше её загрузки). Остальное — фон: в «Прочее».
  */
  const groups = useMemo<HostGroup[]>(() => {
    const token = siteToken(site);
    const map = new Map<string, SeenHost[]>();
    for (const h of hosts) {
      const d = baseDomain(h.host);
      map.set(d, [...(map.get(d) ?? []), h]);
    }
    return [...map.entries()]
      .map(([domain, hs]) => {
        const other = sites.find((s) => s.id !== site.id && s.enabled && s.domains.some((d) => covers(d, domain)));
        const inSite = site.domains.some((d) => covers(d, domain));
        const first = Math.min(...hs.map((h) => h.first_ms));
        const noise = hs.every((h) => NOISE.some((n) => covers(n, h.host)));
        const named = !!token && hs.some((h) => h.host.includes(token));
        return {
          domain,
          hosts: hs,
          count: hs.reduce((n, h) => n + h.count, 0),
          proxied: hs.every((h) => h.via === "proxy"),
          inSite,
          otherSite: other?.name ?? null,
          related: !inSite && !noise && (named || (pageAt !== null && first >= pageAt - BEFORE_PAGE_MS)),
        };
      })
      .sort((a, b) => Number(a.inSite) - Number(b.inSite) || b.count - a.count);
  }, [hosts, site, sites, pageAt]);

  const related = groups.filter((g) => g.related);
  const own = groups.filter((g) => g.inSite);
  const other = groups.filter((g) => !g.related && !g.inSite);
  const candidates = related;
  const toggle = (d: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(d)) next.delete(d);
      else next.add(d);
      return next;
    });

  const add = async () => {
    setAdding(true);
    if (await onAdd([...picked])) setPicked(new Set());
    setAdding(false);
  };

  return createPortal(
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.18 }}
      onMouseDown={onClose}
      className="fixed inset-x-0 bottom-0 top-9 z-[70] flex items-center justify-center bg-black/45 p-5 backdrop-blur-[6px]"
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.94, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: "spring", bounce: 0.18, duration: 0.42 }}
        onMouseDown={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={`Подсмотреть адреса ${site.name}`}
        className="flex max-h-full w-full max-w-[440px] flex-col overflow-hidden rounded-2xl border border-vb-border bg-vb-surface shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
      >
        <div className="flex shrink-0 items-start gap-3 p-4 pb-3">
          <div className="min-w-0 flex-1">
            <div className="truncate text-[16px] font-semibold text-vb-fg">Подсмотреть: {site.name}</div>
            <div className="text-[12px] leading-snug text-vb-silver-faint">
              {mark === null
                ? "Начните запись, откройте сайт в браузере и полистайте его. Запись идёт до 3 минут"
                : expired
                  ? "Запись закончилась (3 минуты) — нажмите «Заново», чтобы записать ещё"
                  : "Записываю — обновите страницу сайта и полистайте, адреса появятся здесь"}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Закрыть"
            className="rounded-full bg-vb-surface-2 p-1.5 text-vb-silver-faint transition-colors hover:text-vb-silver"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4">
          {!browserReady && (
            <p className="mb-3 rounded-lg border border-vb-warn/35 bg-vb-warn/[0.05] px-3 py-2 text-[12px] leading-snug text-vb-warn">
              Сайты не включены ни в одном браузере — запись будет пустой. Включите «Во всех браузерах» на
              странице «Сайты»
            </p>
          )}
          {limited && (
            <p className="mb-3 rounded-lg bg-vb-surface-2/50 px-3 py-2 text-[12px] leading-snug text-vb-silver-dim">
              В режиме «Только в выбранных» запись видит лишь адреса, которые уже идут через прокси. Чтобы найти
              новые, на время переключитесь «Во всех браузерах»
            </p>
          )}
          {err && <p className="mb-3 text-[12px] text-vb-loss">{err}</p>}
          {mark === null ? (
            <div className="flex justify-center py-6">
              <Button onClick={() => void start()}>
                <Eye className="h-4 w-4" />
                Начать запись
              </Button>
            </div>
          ) : groups.length === 0 ? (
            <div className="flex items-center justify-center gap-2 py-8 text-[12px] text-vb-silver-faint">
              <span className="relative flex h-2 w-2">
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-vb-emerald/60" />
                <span className="relative inline-flex h-2 w-2 rounded-full bg-vb-emerald" />
              </span>
              Жду запросов браузера…
            </div>
          ) : (
            <>
              {pageAt === null && (
                <p className="mb-3 rounded-lg bg-vb-surface-2/50 px-3 py-2 text-[12px] leading-snug text-vb-silver-dim">
                  Обновите страницу {site.domains[0]} в браузере — по ней видно, какие адреса нужны сайту
                </p>
              )}
              {related.length + own.length > 0 && (
                <div className="divide-y divide-vb-border/60 overflow-hidden rounded-xl bg-vb-surface-2/50">
                  {[...related, ...own].map(renderGroup)}
                </div>
              )}
              {pageAt !== null && related.length === 0 && (
                <p className="mt-2 px-1 text-[12px] text-vb-silver-faint">
                  Других адресов, нужных сайту, пока нет — полистайте его, откройте видео или вход
                </p>
              )}
              {other.length > 0 && (
                <div className="mt-3 pb-3">
                  <button
                    type="button"
                    onClick={() => setShowOther((v) => !v)}
                    className="flex items-center gap-1.5 px-1 text-[12px] text-vb-silver-faint transition-colors hover:text-vb-silver"
                  >
                    <ChevronDown className={cn("h-3.5 w-3.5 transition-transform duration-200", showOther && "rotate-180")} />
                    Прочее: {other.length} — другие вкладки, программы, реклама
                  </button>
                  {showOther && (
                    <div className="mt-2 divide-y divide-vb-border/60 overflow-hidden rounded-xl bg-vb-surface-2/50">
                      {other.map(renderGroup)}
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>

        <div className="flex shrink-0 items-center justify-between gap-2 border-t border-vb-border/70 p-4 pt-3">
          {mark !== null ? (
            <button
              type="button"
              onClick={() =>
                setPicked(new Set(candidates.filter((g) => !g.proxied).map((g) => g.domain)))
              }
              disabled={candidates.every((g) => g.proxied)}
              className="text-[12px] text-vb-silver-dim transition-colors hover:text-vb-silver disabled:opacity-40"
            >
              Отметить нужные
            </button>
          ) : (
            <span />
          )}
          <div className="flex gap-2">
            {mark !== null && (
              <Button variant="ghost" size="sm" onClick={() => void start()}>
                Заново
              </Button>
            )}
            <Button size="sm" disabled={picked.size === 0 || adding} onClick={() => void add()}>
              {adding && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              Добавить к сайту{picked.size > 0 ? ` (${picked.size})` : ""}
            </Button>
          </div>
        </div>
      </motion.div>
    </motion.div>,
    document.body,
  );

  function renderGroup(g: HostGroup) {
                const selectable = !g.inSite;
                const on = picked.has(g.domain);
                return (
                  <button
                    key={g.domain}
                    type="button"
                    disabled={!selectable}
                    onClick={() => toggle(g.domain)}
                    className={cn(
                      "flex w-full items-center gap-3 px-3 py-2 text-left transition-colors",
                      selectable && "hover:bg-vb-surface-2/80",
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-4 w-4 shrink-0 items-center justify-center rounded border",
                        g.inSite
                          ? "border-vb-emerald/40 bg-vb-emerald/15"
                          : on
                            ? "border-vb-emerald bg-vb-emerald"
                            : "border-vb-border-strong",
                      )}
                    >
                      {(g.inSite || on) && <Check className={cn("h-3 w-3", on ? "text-black" : "text-vb-emerald")} />}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-mono text-[12px] text-vb-fg">{g.domain}</span>
                      <span className="block truncate text-[10px] text-vb-silver-faint">
                        {g.hosts.length > 1 ? `${g.hosts.length} адресов · ` : g.hosts[0].host !== g.domain ? `${g.hosts[0].host} · ` : ""}
                        {g.count} запр.
                        {g.inSite ? " · уже в сайте" : g.otherSite ? ` · в «${g.otherSite}»` : ""}
                      </span>
                    </span>
                    <span
                      className={cn(
                        "shrink-0 text-[10px]",
                        g.proxied ? "text-vb-emerald" : "text-vb-warn",
                      )}
                    >
                      {g.proxied ? "через прокси" : "напрямую"}
                    </span>
                  </button>
                );
  }
}
