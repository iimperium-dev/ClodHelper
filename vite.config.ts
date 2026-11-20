import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import fs from "node:fs";
import path from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
/**
 * Карты исходников нужны нам для разбора сбоев, а не пользователям: после
 * сборки переносим все .map из dist в sourcemaps/<версия>/ (Tauri упаковывает
 * dist целиком, и карты попали бы в программу).
 */
function moveSourcemaps(): Plugin {
  return {
    name: "move-sourcemaps",
    apply: "build",
    closeBundle() {
      const root = process.cwd();
      const dist = path.join(root, "dist");
      if (!fs.existsSync(dist)) return;
      const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version as string;
      const out = path.join(root, "sourcemaps", version);
      const walk = (dir: string) => {
        for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
          const full = path.join(dir, e.name);
          if (e.isDirectory()) walk(full);
          else if (e.name.endsWith(".map")) {
            const dest = path.join(out, path.relative(dist, full));
            fs.mkdirSync(path.dirname(dest), { recursive: true });
            fs.renameSync(full, dest);
          }
        }
      };
      walk(dist);
    },
  };
}

export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), moveSourcemaps()],

  // Скрытые карты: файл есть, но в бандле нет ссылки на него.
  build: { sourcemap: "hidden" },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
