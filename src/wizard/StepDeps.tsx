import { useEffect, useState } from "react";
import {
  Loader2,
  CheckCircle2,
  XCircle,
  ExternalLink,
  RefreshCw,
} from "lucide-react";
import { Button } from "../components/Button";
import type { InstallMode, NodeInfo, PythonInfo } from "../lib/types";
import { detectNode, detectPython } from "../lib/api";
import { openUrl } from "@tauri-apps/plugin-opener";

interface Props {
  mode: InstallMode | null;
  cursor: boolean;
  node: NodeInfo | null;
  python: PythonInfo | null;
  onChange: (node: NodeInfo, python: PythonInfo) => void;
  onBack: () => void;
  onNext: () => void;
}

export function StepDeps({
  mode,
  cursor,
  node,
  python,
  onChange,
  onBack,
  onNext,
}: Props) {
  const [busy, setBusy] = useState(false);

  const needNode = mode === "code" || mode === "both";
  // Мост (local-proxy.py) нужен любому варианту, кроме «только Claude Code»
  // (там тоже нужен, но это покрывалось ранее). Для Desktop и Cursor — Python.
  const needPython = mode === "desktop" || mode === "both" || cursor;

  async function runDetect() {
    setBusy(true);
    try {
      const [n, p] = await Promise.all([detectNode(), detectPython()]);
      onChange(n, p);
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (!node && !python) runDetect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const nodeOk = !needNode || node?.installed === true;
  const pythonOk = !needPython || python?.installed === true;
  const allOk = nodeOk && pythonOk;

  return (
    <div className="mx-auto max-w-2xl pt-4">
      <h2 className="text-[22px] font-semibold tracking-tight text-vb-silver">
        Проверяем зависимости
      </h2>
      <p className="mt-1.5 text-[13px] text-vb-silver-dim">
        Claude использует бесплатные программы от Microsoft и Python.org. Если
        чего-то нет — установите по ссылке и нажмите «Перепроверить».
      </p>

      <div className="mt-7 space-y-3">
        {needNode && (
          <DepRow
            label="Node.js"
            sublabel="нужен для Claude в терминале"
            state={
              busy
                ? "checking"
                : node?.installed
                  ? "ok"
                  : node
                    ? "missing"
                    : "checking"
            }
            version={node?.version}
            downloadUrl="https://nodejs.org/en/download"
          />
        )}
        {needPython && (
          <DepRow
            label="Python 3"
            sublabel="нужен для моста Claude Desktop"
            state={
              busy
                ? "checking"
                : python?.installed
                  ? "ok"
                  : python
                    ? "missing"
                    : "checking"
            }
            version={python?.version}
            downloadUrl="https://www.python.org/downloads/"
          />
        )}
      </div>

      <div className="mt-6 flex justify-center">
        <Button variant="ghost" onClick={runDetect} disabled={busy}>
          <RefreshCw className="h-4 w-4" />
          Перепроверить
        </Button>
      </div>

      <div className="mt-6 flex items-center justify-between">
        <Button variant="ghost" onClick={onBack}>
          Назад
        </Button>
        <Button onClick={onNext} disabled={!allOk || busy}>
          Дальше
        </Button>
      </div>
    </div>
  );
}

function DepRow({
  label,
  sublabel,
  state,
  version,
  downloadUrl,
}: {
  label: string;
  sublabel: string;
  state: "checking" | "ok" | "missing";
  version?: string;
  downloadUrl: string;
}) {
  return (
    <div className="flex items-center gap-4 rounded-xl border border-vb-border bg-vb-surface/40 p-4">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center">
        {state === "checking" && (
          <Loader2 className="h-5 w-5 animate-spin text-vb-silver-dim" />
        )}
        {state === "ok" && (
          <CheckCircle2 className="h-5 w-5 text-vb-emerald" />
        )}
        {state === "missing" && <XCircle className="h-5 w-5 text-vb-loss" />}
      </div>
      <div className="flex-1">
        <div className="text-[14px] font-medium text-vb-silver">{label}</div>
        <div className="text-[12px] text-vb-silver-dim">
          {state === "ok" && version ? (
            <span className="font-mono">{version}</span>
          ) : (
            sublabel
          )}
        </div>
      </div>
      {state === "missing" && (
        <Button
          variant="secondary"
          onClick={() => openUrl(downloadUrl)}
          className="text-[12px]"
        >
          Скачать
          <ExternalLink className="h-3 w-3" />
        </Button>
      )}
    </div>
  );
}
