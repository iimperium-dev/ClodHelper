import { useEffect, useState } from "react";
import { WizardShell } from "./components/WizardShell";
import { StepWelcome } from "./wizard/StepWelcome";
import { StepMode } from "./wizard/StepMode";
import { StepProxy } from "./wizard/StepProxy";
import { StepDeps } from "./wizard/StepDeps";
import { StepInstall } from "./wizard/StepInstall";
import { Dashboard } from "./pages/Dashboard";
import { Settings } from "./pages/Settings";
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

function App() {
  // ─── Top-level routing ───────────────────────────────────────
  // На старте определяем: установлен ли уже шим (есть proxy_url в settings).
  // - есть → Dashboard
  // - нет  → Wizard
  // `null` пока идёт загрузка → пустой экран (без мигания).
  const [view, setView] = useState<AppView | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        // 3 признака что шим уже установлен (хватит любого):
        //   1) есть proxy_url в settings.json
        //   2) на диске лежит local-proxy.py в %LOCALAPPDATA%/ClaudeDesktopProxy
        //   3) шим-процесс прямо сейчас запущен
        // Это нужно для legacy-юзеров, которые ставили clod-helper ДО того
        // как мы научили его писать settings.json — у них нет файла, но шим
        // фактически работает, и Wizard им показывать не надо.
        const [s, scriptPath, status] = await Promise.all([
          settingsGet().catch(() => null),
          shimScriptPath().catch(() => null),
          shimStatus().catch(() => null),
        ]);

        const installed =
          !!s?.proxy_url || !!scriptPath || !!status?.running;
        setView(installed ? "dashboard" : "wizard");
      } catch {
        setView("wizard");
      }
    })();
  }, []);

  // ─── Wizard state (legacy) ───────────────────────────────────
  const [step, setStep] = useState<WizardStep>("welcome");
  const [mode, setMode] = useState<InstallMode | null>(null);
  const [cursor, setCursor] = useState(false);
  const [proxy, setProxy] = useState<ProxyConfig | null>(null);
  const [proxyCheck, setProxyCheck] = useState<ProxyCheckResult | null>(null);
  const [node, setNode] = useState<NodeInfo | null>(null);
  const [python, setPython] = useState<PythonInfo | null>(null);

  if (view === null) {
    return <div className="h-screen w-screen bg-black" />;
  }

  if (view === "dashboard") {
    return <Dashboard onOpenSettings={() => setView("settings")} />;
  }

  if (view === "settings") {
    return <Settings onBack={() => setView("dashboard")} />;
  }

  // view === "wizard"
  return (
    <WizardShell step={step}>
      {step === "welcome" && (
        <StepWelcome onNext={() => setStep("mode")} />
      )}
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
          onDone={() => {
            // После завершения мастера — переходим в Dashboard.
            setView("dashboard");
          }}
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
  );
}

export default App;
