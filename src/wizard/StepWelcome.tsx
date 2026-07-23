import { Sparkles, Shield, Globe } from "lucide-react";
import { Button } from "../components/Button";

interface Props {
  onNext: () => void;
}

export function StepWelcome({ onNext }: Props) {
  return (
    <div className="flex h-full flex-col items-center justify-center text-center">
      <div className="glass-card mx-auto max-w-xl p-10">
        <div className="mx-auto mb-5 flex h-14 w-14 items-center justify-center rounded-[10px] bg-vb-emerald/10">
          <Sparkles className="h-7 w-7 text-vb-emerald" strokeWidth={1.5} />
        </div>
        <h1 className="text-[28px] font-semibold tracking-tight text-vb-silver">
          Подключим Claude через ваш прокси
        </h1>
        <p className="mx-auto mt-3 max-w-md text-[14px] leading-relaxed text-vb-silver-dim">
          Займёт пару минут. Мы всё сделаем сами — вам нужна только строка
          прокси, которую вы купили.
        </p>

        <div className="mt-8 grid grid-cols-1 gap-3 text-left sm:grid-cols-2">
          <Feature
            icon={<Shield className="h-4 w-4" />}
            title="Не трогаем систему"
            body="Прокси работает только для Claude, остальной интернет — напрямую"
          />
          <Feature
            icon={<Globe className="h-4 w-4" />}
            title="В обход блокировок"
            body="Anthropic API не отвечает на российские IP — нужен зарубежный прокси"
          />
        </div>

        <div className="mt-9">
          <Button onClick={onNext} className="px-7">
            Поехали
          </Button>
        </div>

        <button
          type="button"
          onClick={() =>
            import("@tauri-apps/plugin-opener").then((m) =>
              m.openUrl("https://proxy6.net/?r=692907"),
            )
          }
          className="mt-6 text-[11px] text-vb-silver-dim/60 transition-colors hover:text-vb-silver-dim"
        >
          Ещё нет прокси? Берите на proxy6.net (поддержит проект)
        </button>
      </div>
    </div>
  );
}

function Feature({
  icon,
  title,
  body,
}: {
  icon: React.ReactNode;
  title: string;
  body: string;
}) {
  return (
    <div className="rounded-lg border border-vb-border bg-vb-surface/40 p-4">
      <div className="flex items-center gap-2 text-vb-silver">
        <span className="text-vb-emerald">{icon}</span>
        <span className="text-[13px] font-medium">{title}</span>
      </div>
      <p className="mt-1.5 text-[12px] leading-relaxed text-vb-silver-dim">
        {body}
      </p>
    </div>
  );
}
