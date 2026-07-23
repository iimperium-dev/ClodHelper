import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Titlebar } from "./components/Titlebar";
import { Sidebar, NAV_ORDER } from "./components/Sidebar";
import { WizardShell } from "./components/WizardShell";
import { StepWelcome } from "./wizard/StepWelcome";
import { StepMode } from "./wizard/StepMode";
import { StepProxy } from "./wizard/StepProxy";
import { StepDeps } from "./wizard/StepDeps";
import { StepInstall } from "./wizard/StepInstall";
import { Overview } from "./pages/Overview";
import { Proxies } from "./pages/Proxies";
import { Apps } from "./pages/Apps";
import { Settings } from "./pages/Settings";
import { TrayPopup } from "./pages/TrayPopup";
import { StatusProvider } from "./lib/status";
import { settingsGet, shimScriptPath, shimStatus } from "./lib/api";
import type {
  AppView,
  InstallMode,
  NodeInfo,
  ProxyCheckResult,
  ProxyConfig,
  PythonInfo,
  WizardStep,
} from "./lib/types";

/** Окно трей-попапа рендерит тот же bundle с ?view=tray. */
const IS_TRAY = new URLSearchParams(window.location.search).get("view") === "tray";

/*
  Direction-aware переход между разделами: контент уезжает в сторону,
  противоположную движению по навигации. Тайминги короткие (exit 100ms +
  enter 160ms): mode="wait" играет их ПОСЛЕДОВАТЕЛЬНО, длинные тайминги
  ощущались как «долго листается».
*/
const viewVariants = {
  enter: (dir: number) => ({ opacity: 0, y: 14 * dir }),
  center: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.16, ease: [0.22, 1, 0.36, 1] as const },
  },
  exit: (dir: number) => ({
    opacity: 0,
    y: -10 * dir,
    transition: { duration: 0.1, ease: [0.22, 1, 0.36, 1] as const },
  }),
};

function App() {
  // ─── Top-level routing ───────────────────────────────────────
  // На старте определяем: установлен ли уже мост → Обзор, иначе Wizard.
  const [view, setView] = useState<AppView | null>(IS_TRAY ? "overview" : null);
  const prevIndex = useRef(0);
  const [direction, setDirection] = useState(1);

  useEffect(() => {
    if (IS_TRAY) return;
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

  const navigate = (next: AppView) => {
    const nextIdx = NAV_ORDER.indexOf(next);
    const prevIdx = prevIndex.current;
    setDirection(nextIdx >= prevIdx ? 1 : -1);
    prevIndex.current = nextIdx;
    setView(next);
  };

  // ─── Wizard state (legacy) ───────────────────────────────────
  const [step, setStep] = useState<WizardStep>("welcome");
  const [mode, setMode] = useState<InstallMode | null>(null);
  const [cursor, setCursor] = useState(false);
  const [proxy, setProxy] = useState<ProxyConfig | null>(null);
  const [proxyCheck, setProxyCheck] = useState<ProxyCheckResult | null>(null);
  const [node, setNode] = useState<NodeInfo | null>(null);
  const [python, setPython] = useState<PythonInfo | null>(null);

  // ─── Трей-попап: отдельная вьюха в прозрачном окне ───────────
  if (IS_TRAY) {
    return (
      <StatusProvider>
        <TrayPopup />
      </StatusProvider>
    );
  }

  // ─── Wizard (первый запуск) ──────────────────────────────────
  // StatusProvider и здесь: логотип в titlebar показывает живой статус.
  if (view === "wizard") {
    return (
      <StatusProvider>
        <div className="flex h-screen w-screen flex-col overflow-hidden">
          <Titlebar />
        <div className="min-h-0 flex-1">
          <WizardShell step={step}>
            {step === "welcome" && <StepWelcome onNext={() => setStep("mode")} />}
            {step === "mode" && (
              <StepMode
                value={mode}
                cursor={cursor}
                onChange={setMode}
                onCursorChange={setCursor}
                onBack={() => setStep("welcome")}
                onNext={() => setStep("proxy")}
              />
            )}
            {step === "proxy" && (
              <StepProxy
                proxy={proxy}
                check={proxyCheck}
                onChange={(p, c) => {
                  setProxy(p);
                  setProxyCheck(c);
                }}
                onBack={() => setStep("mode")}
                onNext={() => setStep("deps")}
              />
            )}
            {step === "deps" && (mode || cursor) && (
              <StepDeps
                mode={mode}
                cursor={cursor}
                node={node}
                python={python}
                onChange={(n, p) => {
                  setNode(n);
                  setPython(p);
                }}
                onBack={() => setStep("proxy")}
                onNext={() => setStep("install")}
              />
            )}
            {step === "install" && (mode || cursor) && proxy && (
              <StepInstall
                mode={mode}
                cursor={cursor}
                proxyUrl={proxy.url}
                onBack={() => setStep("deps")}
                onDone={() => setView("overview")}
                onRestartWithCode={
                  mode === "desktop"
                    ? () => {
                        setMode("code");
                        setStep("mode");
                      }
                    : undefined
                }
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
        <div className="flex min-h-0 flex-1">
          {view !== null && <Sidebar view={view} onNavigate={navigate} />}
          <main className="min-h-0 flex-1 overflow-y-auto">
            <AnimatePresence mode="wait" custom={direction}>
              <motion.div
                key={view ?? "loading"}
                custom={direction}
                variants={viewVariants}
                initial="enter"
                animate="center"
                exit="exit"
                className="min-h-full"
              >
                {view === "overview" ? (
                  <Overview onOpenProxies={() => navigate("proxies")} />
                ) : view === "proxies" ? (
                  <Proxies />
                ) : view === "apps" ? (
                  <Apps />
                ) : view === "settings" ? (
                  <Settings />
                ) : (
                  <div className="h-full w-full" />
                )}
              </motion.div>
            </AnimatePresence>
          </main>
        </div>
      </div>
    </StatusProvider>
  );
}

export default App;
