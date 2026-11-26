import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import { Check, Copy, ExternalLink, Loader2, ShieldAlert } from "lucide-react";
import { Button } from "./Button";
import { vpnHappAdd } from "../lib/api";
import { useStatus } from "../lib/status";
import { useDialogFocus } from "../lib/useDialogFocus";

/*
  VPN не пускает мост к прокси мимо себя. Окно открывается кликом по
  уведомлению «… не пускает мимо себя» (событие Tauri «vpn:help») или
  кнопкой в предупреждении на Обзоре (openVpnHelp). Внутри - как добавить
  адреса прокси в исключения VPN, а для Happ - одна кнопка.
*/

const OPEN_EVENT = "clod:vpn-help";

/** Открыть окно из любого места главного окна. */
export function openVpnHelp() {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

/** Адреса прокси пула без повторов: из пинга, а до него - из моста. */
function useProxyHosts(): string[] {
  const { poolPing, health } = useStatus();
  const fromPing = poolPing.map((p) => p.host);
  const fromBridge = (health?.upstreams ?? []).map((u) => {
    try {
      return new URL(u.url).hostname.replace(/^\[|\]$/g, "");
    } catch {
      return "";
    }
  });
  return [...new Set((fromPing.length ? fromPing : fromBridge).filter(Boolean))];
}

export function VpnHelpHost() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const show = () => setOpen(true);
    window.addEventListener(OPEN_EVENT, show);
    const un = listen("vpn:help", show);
    return () => {
      window.removeEventListener(OPEN_EVENT, show);
      void un.then((f) => f()).catch(() => {});
    };
  }, []);
  return <VpnHelp open={open} onClose={() => setOpen(false)} />;
}

function VpnHelp({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { vpn } = useStatus();
  const hosts = useProxyHosts();
  const dialogRef = useRef<HTMLDivElement>(null);
  const okRef = useRef<HTMLButtonElement>(null);
  useDialogFocus(open, dialogRef, okRef, onClose);
  const name = vpn?.name ?? null;

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          key="vpn-help"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          onMouseDown={onClose}
          className="fixed inset-x-0 bottom-0 top-9 z-[85] flex items-center justify-center bg-black/50 p-5 backdrop-blur-[6px]"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.94, y: 14 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 6 }}
            transition={{ type: "spring", bounce: 0.22, duration: 0.5 }}
            onMouseDown={(e) => e.stopPropagation()}
            ref={dialogRef}
            role="dialog"
            aria-modal="true"
            aria-label="Исключения VPN"
            className="relative flex max-h-full w-full max-w-[440px] flex-col overflow-hidden rounded-2xl border border-vb-border bg-vb-surface shadow-[0_24px_64px_rgba(0,0,0,0.6)]"
          >
            <div className="min-h-0 overflow-y-auto px-6 pb-2 pt-6">
              <div className="flex items-start gap-3.5">
                <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl border border-vb-warn/30 bg-vb-warn/10">
                  <ShieldAlert className="h-5 w-5 text-vb-warn" />
                </span>
                <div className="min-w-0">
                  <h2 className="text-[16px] font-semibold tracking-[-0.01em] text-vb-fg">
                    Добавьте прокси в исключения VPN
                  </h2>
                  <p className="mt-1 text-[12.5px] leading-snug text-vb-silver">
                    {name ? `VPN ${name}` : "Ваш VPN"} не пускает Clod Helper к прокси напрямую. Исключения - это
                    адреса, к которым VPN пускает мимо себя.
                  </p>
                </div>
              </div>

              <ol className="mt-5 space-y-4">
                <Step n={1} title="Откройте настройки VPN">
                  Раздел может называться «Исключения», «Маршрутизация», «Раздельное туннелирование» или «Обход».
                </Step>
                <Step n={2} title="Добавьте туда адреса прокси">
                  <HostList hosts={hosts} />
                </Step>
                <Step n={3} title="Переподключите VPN">
                  Clod Helper сам заметит, что прокси снова отвечают.
                </Step>
              </ol>

              <HappBlock sure={name === "Happ"} hosts={hosts} />
            </div>

            <div className="flex justify-end px-6 pb-5 pt-4">
              <Button ref={okRef} variant="secondary" size="sm" onClick={onClose}>
                Понятно
              </Button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

function Step({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="mt-px grid h-5 w-5 shrink-0 place-items-center rounded-full bg-vb-surface-2 font-mono text-[11px] text-vb-silver">
        {n}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-vb-fg">{title}</div>
        <div className="mt-0.5 text-[12px] leading-snug text-vb-silver-dim">{children}</div>
      </div>
    </li>
  );
}

function HostList({ hosts }: { hosts: string[] }) {
  const [copied, setCopied] = useState(false);
  if (hosts.length === 0) return <>Адреса прокси - в разделе «Прокси».</>;
  const copy = () => {
    void navigator.clipboard.writeText(hosts.join(", ")).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    });
  };
  return (
    <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
      {hosts.map((h) => (
        <code
          key={h}
          className="rounded-md border border-vb-border bg-vb-surface-2/60 px-2 py-0.5 font-mono text-[12px] text-vb-silver"
        >
          {h}
        </code>
      ))}
      <Button variant="ghost" size="sm" onClick={copy} aria-label="Скопировать адреса прокси">
        {copied ? <Check className="h-3.5 w-3.5 text-vb-emerald" /> : <Copy className="h-3.5 w-3.5" />}
        {copied ? "Скопировано" : "Скопировать"}
      </Button>
    </div>
  );
}

/*
  Happ настраивается ссылкой: правило маршрутизации «к прокси - напрямую».
  Знаем, что VPN - Happ (по адаптеру или процессу), - сразу кнопка. Не знаем -
  сначала спросим: в другом VPN ссылка ничего не сделает.
*/
function HappBlock({ sure, hosts }: { sure: boolean; hosts: string[] }) {
  const [asked, setAsked] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (hosts.length === 0) return null;
  const add = async () => {
    setBusy(true);
    setError(null);
    try {
      await vpnHappAdd(hosts);
      setDone(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div data-testid="vpn-happ" className="mt-5 rounded-xl border border-vb-border bg-vb-surface-2/40 px-4 py-3.5">
      {!sure && !asked ? (
        <div className="flex items-center justify-between gap-3">
          <div className="text-[12.5px] text-vb-silver">
            <span className="font-medium text-vb-fg">Ваш VPN - Happ?</span> Его можно настроить одной кнопкой.
          </div>
          <Button variant="secondary" size="sm" onClick={() => setAsked(true)}>
            Да, Happ
          </Button>
        </div>
      ) : (
        <>
          <div className="text-[13px] font-medium text-vb-fg">
            {sure ? "Happ можно настроить одной кнопкой" : "Добавить исключения в Happ"}
          </div>
          <p className="mt-1 text-[12px] leading-snug text-vb-silver-dim">
            {!sure && "Кнопка сработает, только если VPN правда Happ. "}
            Clod Helper откроет Happ и добавит правило «к прокси напрямую». Оно заменит текущие правила маршрутизации
            Happ.
          </p>
          <p className="mt-1.5 text-[12px] leading-snug text-vb-silver-dim">
            Если у подписки в Happ пометка JSON, правила поменять нельзя. Тогда выключайте VPN, когда работаете с
            Claude, или попросите у продавца подписку без JSON.
          </p>
          {done ? (
            <p className="mt-2.5 flex items-center gap-1.5 text-[12px] text-vb-emerald">
              <Check className="h-3.5 w-3.5" />
              Happ открыт. Подтвердите в нём правило и переподключитесь.
            </p>
          ) : (
            <Button variant="primary" size="sm" className="mt-2.5" onClick={add} disabled={busy}>
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ExternalLink className="h-3.5 w-3.5" />}
              Добавить исключения в Happ
            </Button>
          )}
          {error && <p className="mt-1.5 text-[11px] text-vb-loss">{error}</p>}
        </>
      )}
    </div>
  );
}
