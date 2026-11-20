import { useEffect, useState } from "react";
import { PrivacySwitches } from "./PrivacySwitches";
import { privacyGet, privacySet } from "../lib/api";
import type { PrivacyKey } from "../lib/types";

/** Вариант окна предложения: ?variant=a|b в адресе (без параметра - a). */
export function offerVariant(): "a" | "b" {
  try {
    return new URLSearchParams(window.location.search).get("variant") === "b" ? "b" : "a";
  } catch {
    return "a";
  }
}

/** Вариант a: тихий блок «Ещё с этой версии» - отчёты и статистика, сохраняются сразу. */
export function OfferPrivacyBlock({ open }: { open: boolean }) {
  const [values, setValues] = useState<Record<PrivacyKey, boolean>>({
    auto_update: false,
    crash_reports: true,
    usage_stats: true,
  });
  useEffect(() => {
    if (open) void privacyGet().then((s) => setValues((v) => ({ ...v, ...s }))).catch(() => {});
  }, [open]);
  const change = (key: PrivacyKey, on: boolean) => {
    setValues((v) => ({ ...v, [key]: on }));
    void privacySet(key, on).catch(() => setValues((v) => ({ ...v, [key]: !on })));
  };
  return (
    <div className="mx-auto mt-3 w-full max-w-[440px] border-t border-vb-border/60 pt-3 text-left">
      <p className="mb-2 text-[11px] text-vb-silver-faint">Ещё с этой версии</p>
      <PrivacySwitches
        idPrefix="offer"
        keys={["crash_reports", "usage_stats"]}
        values={values}
        onChange={change}
      />
    </div>
  );
}

/** Вариант b: одна приглушённая строка со ссылкой на настройки. */
export function OfferPrivacyNote({ onSettings }: { onSettings: () => void }) {
  return (
    <p className="mx-auto mt-3 max-w-[300px] text-[11px] leading-snug text-vb-silver-faint">
      С этой версии программа также отправляет отчёты об ошибках и анонимную статистику. Выключить можно{" "}
      <button
        type="button"
        onClick={onSettings}
        className="rounded-sm text-vb-silver-dim underline underline-offset-2 transition-colors hover:text-vb-silver"
      >
        в настройках
      </button>
    </p>
  );
}
