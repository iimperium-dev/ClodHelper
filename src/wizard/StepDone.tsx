import { motion, useReducedMotion } from "framer-motion";
import { AppWindow, ArrowRight, Check, Globe, Monitor, Power } from "lucide-react";
import { Button } from "../components/Button";
import { PrivacySwitches } from "../components/PrivacySwitches";
import { WizardFooter, WizardTitle } from "../components/WizardShell";
import type { PrivacyKey, WizardInstallResult } from "../lib/types";
import { cn } from "../lib/cn";
import { osName } from "../lib/platform";

interface Props {
  result: WizardInstallResult;
  /** Автообновление, отчёты и статистика: по умолчанию включены, человек может снять. */
  privacy: Record<PrivacyKey, boolean>;
  onPrivacy: (key: PrivacyKey, on: boolean) => void;
  onFinish: () => void;
}

/*
  Шаг 3: честный итог. Маршрут «компьютер → мост → внешний IP» — из
  РЕАЛЬНОГО shim-теста (запрос через мост), а не из выбранных галочек: если
  проверка не прошла, говорим прямо и ведём в программу чинить.
  Здесь же - три переключателя (автообновление, отчёты, статистика), все включены.
*/
export function StepDone({ result, privacy, onPrivacy, onFinish }: Props) {
  const ok = result.test.ok;
  const reduce = useReducedMotion();

  return (
    <>
      <WizardTitle
        sub={
          ok
            ? undefined // успех виден по маршруту ниже; место нужно переключателям
            : `${result.test.error ?? "Запрос через мост не прошёл."} Чаще всего прокси не отвечает. Его можно сменить в разделе «Прокси».`
        }
      >
        {ok ? "Всё работает" : "Мост установлен, но проверка не прошла"}
      </WizardTitle>

      {/* Маршрут: этот компьютер → мост → выход в интернет. */}
      <div className="mt-4 flex items-center gap-2 rounded-xl border border-vb-border bg-vb-surface px-4 py-2.5">
        <RouteNode icon={<Monitor className="h-3.5 w-3.5" aria-hidden />} label="Этот компьютер" />
        <RouteLine ok reduce={!!reduce} delay={0.1} />
        <RouteNode icon={<Power className="h-3.5 w-3.5" aria-hidden />} label="Мост" />
        <RouteLine ok={ok} reduce={!!reduce} delay={0.35} />
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden
            className={cn("h-2 w-2 shrink-0 rounded-full", ok ? "bg-vb-emerald" : "bg-vb-warn")}
          />
          <span className="min-w-0">
            <span className="block truncate font-mono text-[12px] text-vb-fg">
              {ok ? (result.test.external_ip ?? "выход через прокси") : "нет ответа"}
            </span>
            {ok && result.test.latency_ms != null && (
              <span className="tnum block text-[11px] text-vb-silver-faint">{result.test.latency_ms} мс</span>
            )}
          </span>
        </div>
      </div>

      <ul className="mt-2">
        <Row icon={<Check className="h-4 w-4 text-vb-emerald" aria-hidden />}>
          Мост запускается сам при входе в {osName}
        </Row>
        {result.env_on ? (
          <Row icon={<Globe className="h-4 w-4 text-vb-emerald" aria-hidden />}>
            Терминал, VS Code и Claude Code уже идут через мост
          </Row>
        ) : (
          <Row icon={<Globe className="h-4 w-4 text-vb-warn" aria-hidden />}>
            Терминал пока идёт напрямую. Включить его можно в «Приложениях», в настройках Claude Desktop (шестерёнка)
          </Row>
        )}
        <Row icon={<AppWindow className="h-4 w-4 text-vb-silver-dim" aria-hidden />}>
          Claude Desktop подключается в разделе «Приложения»
        </Row>
      </ul>

      <div className="mt-2 border-t border-vb-border/60 pt-3">
        <PrivacySwitches dense idPrefix="wizard" values={privacy} onChange={onPrivacy} />
      </div>

      <WizardFooter>
        <Button onClick={onFinish} autoFocus>
          Открыть Clod Helper
          <ArrowRight className="h-4 w-4" aria-hidden />
        </Button>
      </WizardFooter>
    </>
  );
}

function RouteNode({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-[12px] text-vb-silver">
      <span className="flex h-6 w-6 items-center justify-center rounded-md bg-vb-surface-2 text-vb-silver-dim">
        {icon}
      </span>
      <span className="hidden sm:inline">{label}</span>
      <span className="sr-only sm:hidden">{label}</span>
    </span>
  );
}

/** Отрезок маршрута: прорисовывается слева направо (transform), обрыв — пунктир. */
function RouteLine({ ok, reduce, delay }: { ok: boolean; reduce: boolean; delay: number }) {
  return (
    <span aria-hidden className="relative h-px min-w-4 flex-1 overflow-hidden">
      {ok ? (
        <motion.span
          className="absolute inset-0 origin-left bg-vb-emerald-dim"
          initial={reduce ? false : { scaleX: 0 }}
          animate={{ scaleX: 1 }}
          transition={{ delay, duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
        />
      ) : (
        <span className="absolute inset-0 border-t border-dashed border-vb-warn/60" />
      )}
    </span>
  );
}

function Row({ icon, children }: { icon: React.ReactNode; children: React.ReactNode }) {
  return (
    <li className="flex items-start gap-2.5 px-1 py-1 text-[13px] leading-snug text-vb-silver">
      <span className="mt-px shrink-0">{icon}</span>
      <span>{children}</span>
    </li>
  );
}

