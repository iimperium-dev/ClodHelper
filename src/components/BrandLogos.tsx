import { useId, type ReactNode } from "react";
import { siClaude, siCursor, siGithubcopilot, siGooglegemini, siTelegram } from "simple-icons";
import { cn } from "../lib/cn";
import type { ServiceGroup } from "../lib/types";

/*
  Бренд-логотипы сервисов и приложений: точные глифы из simple-icons на
  глянцевых мини-плитках с брендовым свечением («объёмный» вид).
  OpenAI и VS Code из пакета удалены по требованию брендов — их глифы
  нарисованы вручную (узел-лепестки и классическая «лента» соответственно).
*/

export function BrandTile({
  glow,
  disabled,
  children,
}: {
  /** Цвет брендового свечения сверху плитки (oklch с альфой). */
  glow: string;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <span
      className={cn(
        "relative flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-white/[0.07] bg-vb-surface-2 shadow-[0_1px_2px_rgba(0,0,0,0.35)] transition-opacity duration-200",
        disabled && "opacity-50 saturate-50",
      )}
      style={{
        backgroundImage: `radial-gradient(26px 20px at 32% 0%, ${glow}, transparent 72%), linear-gradient(oklch(100% 0 0 / 0.05), transparent 45%)`,
      }}
      aria-hidden
    >
      {children}
    </span>
  );
}

/** Узел OpenAI, упрощённый до 6 дуг-лепестков. */
export function OpenAiKnot({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"}>
      <g stroke="#DCE3E0" strokeWidth="1.7" strokeLinecap="round" fill="none">
        {[0, 60, 120, 180, 240, 300].map((a) => (
          <path
            key={a}
            d="M12 4.6 A 7.4 7.4 0 0 1 18.4 8.3"
            transform={`rotate(${a} 12 12)`}
          />
        ))}
      </g>
    </svg>
  );
}

/** Классическая «лента» VS Code (рисуем сами — иконка изъята из simple-icons). */
export function VsCodeMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"}>
      <path
        fill="#2EA7FF"
        d="M23.15 2.587 18.21.332a1.5 1.5 0 0 0-1.712.29L6.327 9.9 3.899 8.06a1 1 0 0 0-1.276.057L1.12 9.486a1 1 0 0 0 0 1.48L3.9 12l-2.78 2.515a1 1 0 0 0 0 1.48l1.502 1.368a1 1 0 0 0 1.276.057l2.428-1.84L16.498 23.9a1.5 1.5 0 0 0 1.712.29l4.942-2.253A1.5 1.5 0 0 0 24 20.58V3.42a1.5 1.5 0 0 0-.85-1.352zM18 17.28 10.29 12 18 6.72z"
      />
    </svg>
  );
}

export function ClaudeMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"}>
      <path d={siClaude.path} fill="#D97757" />
    </svg>
  );
}

export function TelegramMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"} fill={`#${siTelegram.hex}`}>
      <path d={siTelegram.path} />
    </svg>
  );
}

export function GeminiMark({ className }: { className?: string }) {
  const gid = useId();
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"}>
      <defs>
        <linearGradient id={gid} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="#4E9EFF" />
          <stop offset="100%" stopColor="#B07CFF" />
        </linearGradient>
      </defs>
      <path d={siGooglegemini.path} fill={`url(#${gid})`} />
    </svg>
  );
}

export function CopilotMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"}>
      <path d={siGithubcopilot.path} fill="#C9CFCB" />
    </svg>
  );
}

export function CursorMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className ?? "h-4 w-4"}>
      <path d={siCursor.path} fill="#E8EAE9" />
    </svg>
  );
}

/** Логотип сервиса для «Назначений» (раздел Прокси). */
export function ServiceLogo({ id }: { id: ServiceGroup }) {
  const glow: Record<ServiceGroup, string> = {
    anthropic: "oklch(70% 0.15 45 / 0.24)",
    openai: "oklch(75% 0.11 180 / 0.2)",
    google: "oklch(70% 0.15 285 / 0.24)",
    other_ai: "oklch(82% 0.02 160 / 0.14)",
    telegram: "oklch(72% 0.13 230 / 0.26)",
  };
  return (
    <BrandTile glow={glow[id]}>
      {id === "anthropic" && <ClaudeMark />}
      {id === "openai" && <OpenAiKnot />}
      {id === "google" && <GeminiMark />}
      {id === "other_ai" && <CopilotMark />}
      {id === "telegram" && <TelegramMark />}
    </BrandTile>
  );
}

/** Бренд приложения по имени профиля; null — рисовать generic-иконку. */
export function appBrand(name: string): { glyph: ReactNode; glow: string } | null {
  const n = name.toLowerCase();
  if (n.includes("claude")) {
    return { glyph: <ClaudeMark />, glow: "oklch(70% 0.15 45 / 0.24)" };
  }
  if (n.includes("codex") || n.includes("chatgpt") || n.includes("openai") || n.includes("gpt")) {
    return { glyph: <OpenAiKnot />, glow: "oklch(75% 0.11 180 / 0.2)" };
  }
  if (n.includes("vs code") || n.includes("vscode") || n.includes("visual studio")) {
    return { glyph: <VsCodeMark />, glow: "oklch(70% 0.14 250 / 0.24)" };
  }
  if (n.includes("cursor")) {
    return { glyph: <CursorMark />, glow: "oklch(82% 0.02 160 / 0.16)" };
  }
  if (n.includes("copilot")) {
    return { glyph: <CopilotMark />, glow: "oklch(82% 0.02 160 / 0.14)" };
  }
  if (n.includes("telegram")) {
    return { glyph: <TelegramMark />, glow: "oklch(72% 0.13 230 / 0.26)" };
  }
  if (n.includes("gemini")) {
    return { glyph: <GeminiMark />, glow: "oklch(70% 0.15 285 / 0.24)" };
  }
  return null;
}
