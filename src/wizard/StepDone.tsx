import { motion } from "framer-motion";
import { AlertTriangle, AppWindow, Check, Globe } from "lucide-react";
import { Button } from "../components/Button";
import type { WizardInstallResult } from "../lib/types";
import { cn } from "../lib/cn";
import { osName } from "../lib/platform";

interface Props {
  result: WizardInstallResult;
  onFinish: () => void;
}

/*
  Шаг 3: честный итог. Пиллы — из РЕАЛЬНОГО shim-теста (запрос через мост),
  а не из выбранных галочек: если проверка не прошла, говорим об этом прямо
  и ведём в приложение чинить, а не рисуем зелёное.
*/
export function StepDone({ result, onFinish }: Props) {
  const ok = result.test.ok;
  return (
    <div className="mx-auto max-w-2xl pt-8 text-center">
      <motion.div
        initial={{ scale: 0 }}
        animate={{ scale: 1 }}
        transition={{ type: "spring", stiffness: 380, damping: 22 }}
        className={cn(
          "mx-auto flex h-16 w-16 items-center justify-center rounded-full border",
          ok
            ? "border-vb-emerald/40 bg-vb-emerald/10 text-vb-emerald"
            : "border-vb-warn/40 bg-vb-warn/10 text-vb-warn",
        )}
      >
        {ok ? <Check className="h-8 w-8" /> : <AlertTriangle className="h-7 w-7" />}
      </motion.div>

      <h2 className="mt-5 text-[22px] font-semibold tracking-tight text-vb-fg">
        {ok ? "Всё работает" : "Мост установлен, но проверка не прошла"}
      </h2>
      <p className="mx-auto mt-1.5 max-w-md text-[13px] leading-relaxed text-vb-silver-dim">
        {ok ? (
          <>
            Соединение через мост проверено настоящим запросом
            {result.test.external_ip && (
              <>
                {" — внешний IP "}
                <span className="font-mono text-vb-silver">{result.test.external_ip}</span>
              </>
            )}
            {result.test.latency_ms != null && (
              <span className="tnum"> · {result.test.latency_ms} мс</span>
            )}
            .
          </>
        ) : (
          <>
            {result.test.error ?? "Запрос через мост не прошёл."} Чаще всего это
            значит, что прокси не отвечает — проверьте его в разделе «Прокси».
          </>
        )}
      </p>

      <div className="mx-auto mt-6 max-w-md space-y-2 text-left">
        <div className="flex items-center gap-2.5 rounded-lg border border-vb-border/70 px-3.5 py-2.5 text-[13px] text-vb-silver">
          <Check className="h-4 w-4 shrink-0 text-vb-emerald" />
          Мост установлен и будет запускаться сам при входе в {osName}
        </div>
        <div className="flex items-center gap-2.5 rounded-lg border border-vb-border/70 px-3.5 py-2.5 text-[13px] text-vb-silver">
          {result.env_on ? (
            <>
              <Globe className="h-4 w-4 shrink-0 text-vb-emerald" />
              Системное проксирование включено — терминал, VS Code и Claude Code
              уже идут через мост
            </>
          ) : (
            <>
              <Globe className="h-4 w-4 shrink-0 text-vb-warn" />
              Системное проксирование не включилось — тумблер есть на главном
              экране
            </>
          )}
        </div>
        <div className="flex items-center gap-2.5 rounded-lg border border-vb-border/70 px-3.5 py-2.5 text-[13px] text-vb-silver">
          <AppWindow className="h-4 w-4 shrink-0 text-vb-silver-dim" />
          Claude Desktop, Cursor и другие приложения — в разделе «Приложения»
          (ярлыки с прокси)
        </div>
      </div>

      <div className="mt-8">
        <Button onClick={onFinish}>Открыть Clod Helper</Button>
      </div>
    </div>
  );
}
