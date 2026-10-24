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
import { StatusProvider } from "./lib/status";
import { settingsGet, shimScriptPath, shimStatus, whatsNewTake } from "./lib/api";
import type { AppView, WizardInstallResult, WizardStep } from "./lib/types";

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

function App() {
  // ─── Top-level routing ───────────────────────────────────────
  // На старте определяем: установлен ли уже мост → Обзор, иначе Wizard.
  const [view, setView] = useState<AppView | null>(IS_TRAY ? "overview" : null);
  const prevIndex = useRef(0);
  const [direction, setDirection] = useState(1);
  // Версия для окна «Что нового» (приложение только что обновилось).
  const [whatsNew, setWhatsNew] = useState<string | null>(null);

  useEffect(() => {
    if (IS_TRAY || IS_TOAST || IS_SPLASH) return;
    // До выбора мастер/Обзор: первый запуск после установки ещё без прокси —
    // это не обновление, «Что нового» не нужно.
    void whatsNewTake().then(setWhatsNew).catch(() => {});
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

  const navigate = (next: AppView) => {
    const nextIdx = NAV_ORDER.indexOf(next);
    const prevIdx = prevIndex.current;
    setDirection(nextIdx >= prevIdx ? 1 : -1);
    prevIndex.current = nextIdx;
    setView(next);
  };

  // ─── Wizard state: 3 шага (прокси → установка → готово) ──────
  const [step, setStep] = useState<WizardStep>("proxy");
  const [wizProxyUrl, setWizProxyUrl] = useState<string | null>(null);
  const [wizResult, setWizResult] = useState<WizardInstallResult | null>(null);

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
                <StepDone result={wizResult} onFinish={() => setView("overview")} />
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
    </StatusProvider>
  );
}

export default App;
