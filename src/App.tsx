import { useCallback, useEffect, useRef, useState } from "react";
import { Titlebar } from "./components/Titlebar";
import { Sidebar, NAV_ORDER } from "./components/Sidebar";
import { WizardShell } from "./components/WizardShell";
import { StepProxy } from "./wizard/StepProxy";
import { StepInstall } from "./wizard/StepInstall";
import { StepDone } from "./wizard/StepDone";
import { Overview } from "./pages/Overview";
import { Proxies } from "./pages/Proxies";
import { Apps } from "./pages/Apps";
import { Sites } from "./pages/Sites";
import { Settings } from "./pages/Settings";
import { Tools } from "./pages/Tools";
import { TrayPopup } from "./pages/TrayPopup";
import { UpdateToast } from "./pages/UpdateToast";
import { UpdateSplash } from "./pages/UpdateSplash";
import { UpdateDialog } from "./components/UpdateDialog";
import { WhatsNewModal } from "./components/WhatsNewModal";
import { AutoUpdateOffer } from "./components/AutoUpdateOffer";
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
  whatsNewTake,
} from "./lib/api";
import type { AppView, PrivacyKey, WizardInstallResult, WizardStep } from "./lib/types";

/** Окно трей-попапа рендерит тот же bundle с ?view=tray. */
const IS_TRAY = new URLSearchParams(window.location.search).get("view") === "tray";
/** Уведомление «Доступно обновление» в углу экрана — ?view=toast. */
const IS_TOAST = new URLSearchParams(window.location.search).get("view") === "toast";
/** Заставка на время установки обновления (отдельный процесс) — ?view=updating. */
const IS_SPLASH = new URLSearchParams(window.location.search).get("view") === "updating";

/*
  Переход между разделами — CSS-анимация появления (index.css, .view-enter):
  раздел въезжает со стороны движения по навигации. Раньше это делал
  framer-motion (AnimatePresence mode="wait"), и после долгой работы окна
  контейнер раздела изредка застревал в начальном состоянии opacity 0 —
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
  const [view, setView] = useState<AppView | null>(IS_TRAY ? "overview" : null);
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
    if (IS_TRAY || IS_TOAST || IS_SPLASH) return;
    // До выбора мастер/Обзор: первый запуск после установки ещё без прокси —
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
        setView(installed ? "overview" : "wizard");
      } catch {
        setView("wizard");
      }
    })();
  }, []);

  const closeWhatsNew = useCallback(() => setWhatsNew(null), []);
  const answerAutoUpdate = useCallback(async (enabled: boolean) => {
    await autoUpdateSet(enabled).catch(() => {});
    setOfferAuto(false);
  }, []);

  const navigate = (next: AppView) => {
    const nextIdx = NAV_ORDER.indexOf(next);
    const prevIdx = prevIndex.current;
    setDirection(nextIdx >= prevIdx ? 1 : -1);
    prevIndex.current = nextIdx;
    setView(next);
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
  // Мастер пропущен кнопкой «Сначала очистка»: программа открыта без прокси,
  // сверху полоса «Прокси ещё не добавлен» с возвратом в мастер.
  const [noProxy, setNoProxy] = useState(false);

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

  if (IS_TOAST) return <UpdateToast />;
  if (IS_SPLASH) return <UpdateSplash />;

  // ─── Трей-попап: отдельная вьюха в прозрачном окне ───────────
  if (IS_TRAY) {
    return (
      <StatusProvider>
        <TrayPopup />
      </StatusProvider>
    );
  }

  // ─── Wizard (первый запуск): 3 шага, без легаси-режимов ──────
  // StatusProvider и здесь: логотип в сайдбаре показывает живой статус.
  if (view === "wizard") {
    return (
      <StatusProvider>
        <div className="flex h-screen w-screen flex-col overflow-hidden">
          <Titlebar bell={false} />
          <div className="min-h-0 flex-1">
            <WizardShell step={step}>
              {step === "proxy" && (
                <StepProxy
                  initialUrl={wizProxyUrl}
                  onSkipToTools={() => {
                    setNoProxy(true);
                    prevIndex.current = NAV_ORDER.indexOf("tools");
                    setView("tools");
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
                    setStep("done");
                  }}
                />
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
                    setView("overview");
                  }}
                />
              )}
            </WizardShell>
          </div>
        </div>
      </StatusProvider>
    );
  }

  // ─── App shell: titlebar + sidebar + контент ─────────────────
  return (
    <StatusProvider>
      <div className="flex h-screen w-screen flex-col overflow-hidden">
        <Titlebar />
        {noProxy && (
          <div className="flex items-center gap-3 border-b border-vb-warn/25 bg-vb-warn/[0.07] px-5 py-2.5">
            <AlertTriangle className="h-4 w-4 shrink-0 text-vb-warn" />
            <span className="min-w-0 flex-1 text-[13px] text-vb-silver">
              Прокси ещё не добавлен - Claude пока ходит напрямую. Добавьте его, когда сервер будет готов.
            </span>
            <Button
              size="sm"
              onClick={() => {
                setStep("proxy");
                setView("wizard");
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
              onClick={() => closeBanner(() => navigate("settings"))}
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
            </div>
          </main>
        </div>
      </div>
      <UpdateDialog />
      <WhatsNewModal version={view !== null ? whatsNew : null} onClose={closeWhatsNew} />
      <AutoUpdateOffer
        open={view !== null && whatsNewTaken && !whatsNew && offerAuto}
        onAnswer={answerAutoUpdate}
        onOpenSettings={() => {
          // Без ответа: auto_update остаётся пустым, предложение вернётся при следующем запуске.
          setOfferAuto(false);
          navigate("settings");
        }}
      />
    </StatusProvider>
  );
}

export default App;
