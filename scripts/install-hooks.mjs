// Подключает git-хуки из .githooks (запускается сам при npm install / npm ci).
// Хуки вырезают подпись Claude из коммитов и не дают её запушить.
import { execFileSync } from "node:child_process";

try {
  execFileSync("git", ["config", "core.hooksPath", ".githooks"], { stdio: "ignore" });
} catch {
  // Не git-репозиторий (например, архив исходников) — хуки не нужны.
}
