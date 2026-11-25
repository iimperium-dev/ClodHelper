import { browserPolicyStatus, leakGuardStatus } from "./api";

/** Приложение → сервис защиты от прямого соединения (как guard_service_of в Rust). */
const GUARD: Record<string, string> = { telegram: "telegram", "claude-desktop": "claude", codex: "codex" };

/**
  Что ещё снимет «Убрать из списка» (apps_remove): защиту от прямого
  соединения у Telegram, Claude и Codex и «Всегда через мост» у браузера.
  Строка для подтверждения или null, если снимать нечего. Не узнали про
  защиту - предупреждаем на всякий случай.
*/
export async function removeConsequences(id: string): Promise<string | null> {
  const service = GUARD[id];
  const [guard, policy] = await Promise.all([
    service ? leakGuardStatus().catch(() => null) : Promise.resolve(null),
    browserPolicyStatus(id).catch(() => null),
  ]);
  const notes: string[] = [];
  if (service && (!guard || (guard.enabled && guard.services.split(",").includes(service)))) {
    notes.push("Защита от прямого соединения снимется");
  }
  if (policy?.configured) notes.push("Браузер перестанет всегда ходить через мост");
  return notes.length ? notes.join(". ") : null;
}
