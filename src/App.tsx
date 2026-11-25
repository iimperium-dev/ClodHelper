import { Suspense, useCallback, useEffect, useRef, useState } from "react";
import { Titlebar } from "./components/Titlebar";
import { Sidebar, NAV_ORDER } from "./components/Sidebar";
import { WizardShell } from "./components/WizardShell";
import { LoadingScreen, PageBoundary, useCalmLoading } from "./components/LoadingScreen";
import { UpdateDialogHost } from "./components/UpdateDialogHost";
import {
  Apps,
  Overview,
  Proxies,
  Settings,
  Sites,
  StepApps,
  StepDone,
  StepInstall,
  StepProxy,
  Tools,
  loadPage,
  pageReady,
  preloadPages,
  preloadWizard,
  whenIdle,
} from "./lazyViews";
import { WhatsNewModal } from "./components/WhatsNewModal";
import { AutoUpdateOffer } from "./components/AutoUpdateOffer";
import { ShopClaimDialog, type ShopClaimState } from "./components/ShopClaimDialog";
import { SHOP_KEY_EVENT } from "./lib/shopClaim";
import { listen } from "@tauri-apps/api/event";
import { openPrivacyNext } from "./components/PrivacyCard";
import { StatusProvider } from "./lib/status";
import { Button } from "./components/Button";
import { AlertTriangle, Info } from "lucide-react";
import {
  autoUpdateGet,
  autoUpdateSet,
  privacyGet,
  privacyNoticeSeen,
  privacySet,
  settingsGet,
  shimScriptPath,
  shimStatus,
  shopClaim,
  shopLinkTake,
  whatsNewTake,
  wizardApps,
} from "./lib/api";
import type { AppView, PrivacyKey, WizardApps, WizardInstallResult, WizardStep } from "./lib/types";

/*
  Главное окно. Трей-попап, уведомление и заставка обновления - свои
  корни в main.tsx и этот файл не грузят.
*/

// «Обзор» нужен почти всегда первым - берём его код сразу, пока идут
// вопросы к программе (мост установлен?).
void Overview.preload().catch(() => {});

/*
  Переход между разделами - CSS-анимация появления (index.css, .view-enter):
  раздел въезжает со стороны движения по навигации. Раньше это делал
  framer-motion (AnimatePresence mode="wait"), и после долгой работы окна
  контейнер раздела изредка застревал в начальном состоянии opacity 0 -
  «Обзор» и «Прокси» выглядели чёрными (видно в ui.log). CSS-анимация
  застрять не может: в худшем случае раздел просто появляется без неё.
*/

/**
 * Вызвать `onShown` один раз, когда `active` и окно правда на виду: видимо,
 * в фокусе и так секунду подряд. Скрытое окно (--tray, тихий перезапуск) ждёт
 * visibilitychange или focus.
 */
function useOnceShown(active: boolean, onShown: () => void) {
  const cb = useRef(onShown);
  cb.current = onShown;
  useEffect(() => {
    if (!active) return;
    let timer: number | undefined;
    let done = false;
    const visible = () => document.visibilityState === "visible" && document.hasFocus();
    const arm = () => {
      if (done || timer !== undefined || !visible()) return;
      timer = window.setTimeout(() => {
        timer = undefined;
        if (!done && visible()) {
          done = true;
          cb.current();
        }
      }, 1000);
    };
    window.addEventListener("focus", arm);
    document.addEventListener("visibilitychange", arm);
    arm();
    return () => {
      window.removeEventListener("focus", arm);
      document.removeEventListener("visibilitychange", arm);
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [active]);
}

function App() {
  // ─── Top-level routing ───────────────────────────────────────
  // На старте определяем: установлен ли уже мост → Обзор, иначе Wizard.
  const [view, setView] = useState<AppView | null>(null);
  const prevIndex = useRef(0);
  const [direction, setDirection] = useState(1);
  // Версия для окна «Что нового» (приложение только что обновилось).
  const [whatsNew, setWhatsNew] = useState<string | null>(null);
  // Ответ «Что нового» получен (окно показано или его не будет).
  const [whatsNewTaken, setWhatsNewTaken] = useState(false);
  // Предложение автообновления: для установок до этой функции, один раз,
  // после окна «Что нового» (или сразу, если его нет в этот запуск).
  const [offerAuto, setOfferAuto] = useState(false);
  // Ответ про автообновление получен (окно предложения будет или нет).
  const [offerLoaded, setOfferLoaded] = useState(false);
  // Человек уже видел переключатели или строку про отчёты (null - ещё не знаем).
  const [noticeSeen, setNoticeSeen] = useState<boolean | null>(null);
  // Плашка «отчёты и статистика» в главном окне.
  const [noticeBanner, setNoticeBanner] = useState(false);
  // Установка со старой версии: плашка говорит «С этой версии».
  const [noticeLegacy, setNoticeLegacy] = useState(false);

  useEffect(() => {
    // До выбора мастер/Обзор: первый запуск после установки ещё без прокси -
    // это не обновление, «Что нового» не нужно.
    void whatsNewTake()
      .then(setWhatsNew)
      .catch(() => {})
      .finally(() => setWhatsNewTaken(true));
    void autoUpdateGet()
      .then((s) => setOfferAuto(s.offer))
      .catch(() => {})
      .finally(() => setOfferLoaded(true));
    // Не удалось спросить - считаем, что видел: лишней плашки не нужно.
    void privacyGet()
      .then((s) => {
        setNoticeSeen(s.notice_seen);
        setNoticeLegacy(s.legacy);
      })
      .catch(() => setNoticeSeen(true));
    void (async () => {
      try {
        // 3 признака что мост уже установлен (хватит любого):
        //   1) есть proxy_url в settings.json
        //   2) на диске лежит установленный мост
        //   3) мост-процесс прямо сейчас запущен
        const [s, scriptPath, status] = await Promise.all([
          settingsGet().catch(() => null),
          shimScriptPath().catch(() => null),
          shimStatus().catch(() => null),
        ]);
        const installed = !!s?.proxy_url || !!scriptPath || !!status?.running;
        showView(installed ? "overview" : "wizard");
      } catch {
        showView("wizard");
      }
    })();
  }, []);

  const closeWhatsNew = useCallback(() => setWhatsNew(null), []);
  const answerAutoUpdate = useCallback(async (enabled: boolean) => {
    await autoUpdateSet(enabled).catch(() => {});
    setOfferAuto(false);
  }, []);

  // Раздел показываем, когда его код уже загружен (обычно он взят заранее):
  // тогда он рисуется сразу, без заглушки. Нажали другой раздел, пока
  // грузился первый, - покажется последний.
  const navToken = useRef(0);
  const showView = useCallback((next: AppView) => {
    const t = ++navToken.current;
    if (pageReady(next)) {
      setView(next);
      return;
    }
    void loadPage(next).then(() => {
      if (navToken.current === t) setView(next);
    });
  }, []);

  const navigate = (next: AppView) => {
    const nextIdx = NAV_ORDER.indexOf(next);
    const prevIdx = prevIndex.current;
    setDirection(nextIdx >= prevIdx ? 1 : -1);
    prevIndex.current = nextIdx;
    showView(next);
  };

  // ─── Wizard state: 3 шага (прокси → установка → готово) ──────
  const [step, setStep] = useState<WizardStep>("proxy");
  // Автообновление в мастере: включено по умолчанию, пишется в настройки
  // при старте установки и при каждом переключении на шаге «Готово».
  const [wizPrivacy, setWizPrivacy] = useState<Record<PrivacyKey, boolean>>({
    auto_update: true,
    crash_reports: true,
    usage_stats: true,
  });
  const [wizProxyUrl, setWizProxyUrl] = useState<string | null>(null);
  const [wizResult, setWizResult] = useState<WizardInstallResult | null>(null);
  // Claude Desktop и Codex: чего нет - шаг «Приложения» предложит поставить.
  // Проверяется при входе в мастер (сколько шагов в шапке) и ещё раз после
  // установки моста (человек мог поставить что-то, пока вводил прокси).
  const [wizApps, setWizApps] = useState<WizardApps | null>(null);
  const needApps = !!wizApps && (!wizApps.claude || !wizApps.codex);
  const wizSteps: WizardStep[] = needApps ? ["proxy", "install", "apps", "done"] : ["proxy", "install", "done"];
  // Мастер пропущен кнопкой «Сначала очистка»: программа открыта без прокси,
  // сверху полоса «Прокси ещё не добавлен» с возвратом в мастер.
  const [noProxy, setNoProxy] = useState(false);
  // Поле мастера заполнили снаружи (купленный прокси) - шаг создаётся заново.
  const [wizFill, setWizFill] = useState(0);

  // ─── Купленный прокси: ссылка clodhelper://add?key=... ───────
  // Rust держит ключ, пока окно его не заберёт (холодный старт по ссылке),
  // и шлёт "shop:link" на каждую новую. Ничего не добавляется без кнопки.
  const [shop, setShop] = useState<ShopClaimState | null>(null);
  const shopKey = useRef<string | null>(null);
  const loadClaim = useCallback((key: string) => {
    shopKey.current = key;
    setShop({ kind: "loading" });
    shopClaim(key).then(
      (claim) => shopKey.current === key && setShop({ kind: "ready", claim, key }),
      (e) => shopKey.current === key && setShop({ kind: "error", message: String(e) }),
    );
  }, []);
  const closeShop = useCallback(() => {
    shopKey.current = null;
    setShop(null);
  }, []);
  useEffect(() => {
    const take = () =>
      void shopLinkTake()
        .then((key) => {
          // Пустой ключ - ссылка на покупку пришла повреждённой.
          if (key === "") {
            shopKey.current = null;
            setShop({ kind: "error", message: "Ссылка повреждена: ключ в ней неполный. Скопируйте ключ из письма целиком и вставьте его в поле прокси.", final: true });
          } else if (key) loadClaim(key);
        })
        .catch(() => {});
    take();
    const un = listen("shop:link", take);
    // Ключ вставили в поле прокси (страница «Прокси», мастер).
    const onKey = (e: Event) => loadClaim((e as CustomEvent<string>).detail);
    window.addEventListener(SHOP_KEY_EVENT, onKey);
    return () => {
      window.removeEventListener(SHOP_KEY_EVENT, onKey);
      void un.then((f) => f()).catch(() => {});
    };
  }, [loadClaim]);

  // «Показано» ставится, только когда человек правда мог это увидеть: окно
  // видимо и в фокусе (не --tray и не тихий перезапуск после установки),
  // и ещё секунду после этого. Идемпотентно.
  const markedRef = useRef(false);
  const markSeen = useCallback(() => {
    if (markedRef.current) return;
    markedRef.current = true;
    setNoticeSeen(true);
    void privacyNoticeSeen().catch(() => {});
  }, []);

  // Окно предложения автообновления показано: там есть строка про отчёты.
  const offerOpen = view !== null && view !== "wizard" && whatsNewTaken && !whatsNew && offerAuto;
  useOnceShown(offerOpen, markSeen);

  // Мастер показал «Готово»: записываем выбранные переключатели, потом флаг.
  // До этого момента ничего не пишется и ничего не уходит. Если человек уже
  // отвечал раньше (плашка и настройки), значения взяты из настроек и
  // перезаписывать их нечем: пишутся только его щелчки.
  const wizPrivacyRef = useRef(wizPrivacy);
  wizPrivacyRef.current = wizPrivacy;
  const wizSeenBefore = useRef(false);
  const wizMarked = useRef(false);
  const markWizard = useCallback(() => {
    if (wizMarked.current) return;
    wizMarked.current = true;
    setNoticeSeen(true);
    markedRef.current = true;
    if (wizSeenBefore.current) return;
    void (async () => {
      for (const k of Object.keys(wizPrivacyRef.current) as PrivacyKey[]) {
        await privacySet(k, wizPrivacyRef.current[k]).catch(() => {});
      }
      await privacyNoticeSeen().catch(() => {});
    })();
  }, []);
  const wizDone = view === "wizard" && step === "done" && wizResult !== null;
  useOnceShown(wizDone, markWizard);

  // Вернулись в мастер, а человек уже отвечал (плашка, настройки): берём его выбор.
  useEffect(() => {
    if (view !== "wizard") return;
    void wizardApps()
      .then(setWizApps)
      .catch(() => {});
  }, [view]);
  const onWizClaude = useCallback((claude: WizardInstallResult["claude"], running: boolean) => {
    setWizResult((r) => (r ? { ...r, claude, claude_running: running } : r));
  }, []);
  const toWizDone = useCallback(() => setStep("done"), []);

  useEffect(() => {
    if (view !== "wizard") return;
    wizMarked.current = false;
    void privacyGet()
      .then((s) => {
        wizSeenBefore.current = s.notice_seen;
        if (s.notice_seen) {
          setWizPrivacy({ auto_update: s.auto_update, crash_reports: s.crash_reports, usage_stats: s.usage_stats });
        }
      })
      .catch(() => {});
  }, [view]);

  // Все остальные: главное окно без мастера и без окна предложения. Один раз
  // тихая плашка (не поверх полосы «Прокси ещё не добавлен»); флаг ставится,
  // когда она правда показана.
  const showNotice =
    view !== null &&
    view !== "wizard" &&
    whatsNewTaken &&
    offerLoaded &&
    !whatsNew &&
    !offerAuto &&
    !noProxy &&
    noticeSeen === false;
  useEffect(() => {
    if (showNotice) setNoticeBanner(true);
  }, [showNotice]);
  useOnceShown(noticeBanner, markSeen);
  // Один раз за всё время: окно скрыли (трей, сворачивание) или человек ушёл
  // на другой раздел - плашка убирается и больше не возвращается.
  const bannerView = useRef<AppView | null>(null);
  useEffect(() => {
    if (!noticeBanner) {
      bannerView.current = null;
      return;
    }
    bannerView.current = view;
    const onHide = () => {
      if (document.visibilityState === "hidden") {
        markSeen();
        setNoticeBanner(false);
      }
    };
    document.addEventListener("visibilitychange", onHide);
    return () => document.removeEventListener("visibilitychange", onHide);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [noticeBanner]);
  useEffect(() => {
    if (noticeBanner && bannerView.current !== null && view !== bannerView.current) {
      markSeen();
      setNoticeBanner(false);
    }
  }, [view, noticeBanner, markSeen]);
  const closeBanner = (then?: () => void) => {
    markSeen();
    setNoticeBanner(false);
    then?.();
  };

  // Первая настройка (мастер или программа без прокси): прокси - в поле
  // мастера. Пока мастер ставит мост или показывает «Готово», окно ждёт.
  const shopWizard = view === "wizard" || noProxy;
  const shopDialog = (
    <ShopClaimDialog
      state={view !== null && !(view === "wizard" && step !== "proxy") ? shop : null}
      wizard={shopWizard}
      onRetry={() => shopKey.current && loadClaim(shopKey.current)}
      onClose={closeShop}
      onWizardFill={(url) => {
        closeShop();
        setWizProxyUrl(url);
        setWizFill((n) => n + 1);
        setStep("proxy");
        showView("wizard");
      }}
      onOpenProxies={
        view === "proxies"
          ? undefined
          : () => {
              closeShop();
              navigate("proxies");
            }
      }
    />
  );

  // ─── Запуск: пока не знаем, мастер или Обзор ─────────────────
  // Знак программы и «Проверяю…» - только если ответ задержался дольше
  // 150 мс; появившаяся заглушка не мелькает (держится хотя бы 350 мс).
  const boot = useCalmLoading(view === null);
  // Окно нарисовано - остальные разделы (или шаги мастера) берём заранее,
  // чтобы переход был без пустого кадра.
  const isWizard = view === "wizard";
  const viewKnown = view !== null;
  useEffect(() => {
    if (!viewKnown) return;
    return whenIdle(() => {
      if (isWizard) preloadWizard();
      preloadPages();
    });
  }, [viewKnown, isWizard]);

  /*
    Один StatusProvider на всё окно: и мастер, и программа под ним. После
    «Готово» в мастере опрос не начинается заново, история трафика на
    «Обзоре» не обнуляется.
  */
  let content;
  if (boot.waiting) {
    content = (
      <div className="flex h-screen w-screen flex-col overflow-hidden">
        <Titlebar bell={false} />
        <div className="min-h-0 flex-1">{boot.shown && <LoadingScreen brand immediate />}</div>
      </div>
    );
  } else if (view === "wizard") {
    // ─── Wizard (первый запуск): 3 шага, без легаси-режимов ──────
    content = (
      <>
        <div className="flex h-screen w-screen flex-col overflow-hidden">
          <Titlebar bell={false} />
          <div className="min-h-0 flex-1">
            <WizardShell step={step} steps={wizSteps}>
              <PageBoundary>
              <Suspense fallback={<LoadingScreen label="Загружаю…" />}>
                {step === "proxy" && (
                  <StepProxy
                    key={wizFill}
                    initialUrl={wizProxyUrl}
                    onSkipToTools={() => {
                      setNoProxy(true);
                      prevIndex.current = NAV_ORDER.indexOf("tools");
                      showView("tools");
                    }}
                    onNext={(url) => {
                      setWizProxyUrl(url);
                      setStep("install");
                    }}
                  />
                )}
                {step === "install" && wizProxyUrl && (
                  <StepInstall
                    proxyUrl={wizProxyUrl}
                    onBack={() => setStep("proxy")}
                    onDone={(r) => {
                      setWizResult(r);
                      void wizardApps()
                        .catch(() => wizApps)
                        .then((now) => {
                          if (now) setWizApps(now);
                          setStep(now && (!now.claude || !now.codex) ? "apps" : "done");
                        });
                    }}
                  />
                )}
                {step === "apps" && wizResult && wizApps && (
                  <StepApps initial={wizApps} onClaude={onWizClaude} onDone={toWizDone} />
                )}
                {step === "done" && wizResult && (
                  <StepDone
                    result={wizResult}
                    privacy={wizPrivacy}
                    onPrivacy={(key, on) => {
                      setWizPrivacy((p) => ({ ...p, [key]: on }));
                      void privacySet(key, on).catch(() => {});
                    }}
                    onFinish={() => {
                      markWizard();
                      setNoProxy(false);
                      setOfferAuto(false);
                      showView("overview");
                    }}
                  />
                )}
              </Suspense>
              </PageBoundary>
            </WizardShell>
          </div>
        </div>
        {shopDialog}
      </>
    );
  } else {
    // ─── App shell: titlebar + sidebar + контент ─────────────────
    content = (
      <>
        <div className="flex h-screen w-screen flex-col overflow-hidden">
          <Titlebar />
          {noProxy && (
            <div className="flex items-center gap-3 border-b border-vb-warn/25 bg-vb-warn/[0.07] px-5 py-2.5">
              <AlertTriangle className="h-4 w-4 shrink-0 text-vb-warn" />
              <span className="min-w-0 flex-1 text-[13px] text-vb-silver">
                Прокси ещё не добавлен. Приложения пока ходят напрямую. Добавьте его, когда сервер будет готов.
              </span>
              <Button
                size="sm"
                onClick={() => {
                  setStep("proxy");
                  showView("wizard");
                }}
              >
                Добавить прокси
              </Button>
            </div>
          )}
          {noticeBanner && (
            <div
              role="region"
              aria-label="Отчёты и статистика"
              className="flex items-center gap-3 border-b border-vb-border/60 bg-vb-surface px-5 py-2"
            >
              <Info className="h-4 w-4 shrink-0 text-vb-silver-dim" aria-hidden />
              <span className="min-w-0 flex-1 text-[12px] text-vb-silver">
                {noticeLegacy ? "С этой версии программа отправляет" : "Программа отправляет"} отчёты об ошибках и анонимную
                статистику
              </span>
              <Button variant="ghost" size="sm" onClick={() => closeBanner()}>
                Понятно
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => closeBanner(() => {
                  openPrivacyNext();
                  navigate("settings");
                })}
              >
                Настройки
              </Button>
            </div>
          )}
          <div className="flex min-h-0 flex-1">
            {view !== null && <Sidebar view={view} onNavigate={navigate} />}
            <main className="min-h-0 flex-1 overflow-y-auto">
              <div
                key={view ?? "loading"}
                className="view-enter min-h-full"
                style={{ "--view-dir": direction } as React.CSSProperties}
              >
                <PageBoundary>
                  <Suspense fallback={<LoadingScreen label="Загружаю…" />}>
                    {view === "overview" ? (
                      <Overview onOpenProxies={() => navigate("proxies")} />
                    ) : view === "proxies" ? (
                      <Proxies />
                    ) : view === "apps" ? (
                      <Apps />
                    ) : view === "sites" ? (
                      <Sites onOpenApps={() => navigate("apps")} />
                    ) : view === "tools" ? (
                      <Tools onOpenApps={() => navigate("apps")} />
                    ) : view === "settings" ? (
                      <Settings />
                    ) : (
                      <div className="h-full w-full" />
                    )}
                  </Suspense>
                </PageBoundary>
              </div>
            </main>
          </div>
        </div>
        {shopDialog}
        <WhatsNewModal version={view !== null ? whatsNew : null} onClose={closeWhatsNew} />
        <AutoUpdateOffer
          open={view !== null && whatsNewTaken && !whatsNew && offerAuto}
          onAnswer={answerAutoUpdate}
          onOpenSettings={() => {
            // Без ответа: auto_update остаётся пустым, предложение вернётся при следующем запуске.
            setOfferAuto(false);
            openPrivacyNext();
            navigate("settings");
          }}
        />
      </>
    );
  }

  return (
    <StatusProvider>
      {content}
      {/* Вне веток: просьба открыть обновление во время запуска не теряется
          (в мастере окна обновления нет, как и раньше). Сбой его загрузки
          не трогает остальное окно. */}
      {!isWizard && (
        <PageBoundary silent>
          <UpdateDialogHost />
        </PageBoundary>
      )}
    </StatusProvider>
  );
}

export default App;
