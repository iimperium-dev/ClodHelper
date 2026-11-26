import { useState } from "react";
import { Check, Copy, Loader2, ShieldCheck } from "lucide-react";
import { Button } from "./Button";
import { openVpnHelp } from "./VpnHelp";
import { setGlobalProxyEnv } from "../lib/api";
import { useStatus } from "../lib/status";
import { cn } from "../lib/cn";

/*
  VPN рядом с мостом. Мост ходит к прокси мимо VPN (мост 1.17+): получилось -
  спокойная строка вместо тревоги. VPN не пускает мимо себя, и прокси не
  отвечают - подсказка с окном «как добавить исключение». Тревогу не
  поднимаем, пока всё работает. SOTA VPN - отдельный случай: он стирает настройки
  прокси, и терминал с Claude Code идут мимо моста.
*/
export function VpnNotice({ problems, hosts, className }: { problems: boolean; hosts: string[]; className?: string }) {
  const { vpn, env, setEnv, health } = useStatus();
  const [copied, setCopied] = useState(false);
  const [fixing, setFixing] = useState(false);
  const [fixError, setFixError] = useState<string | null>(null);
  if (!vpn) return null;
  const bypass = vpn.active ? (health?.vpn_bypass ?? null) : null;
  const vpnName = vpn.name ? `VPN ${vpn.name}` : "VPN";
  // Мимо VPN ходим - или не пускает, но прокси отвечают и через него.
  const calm = bypass !== null && (bypass.ok === true || (bypass.ok === false && !problems));
  const tunnel = vpn.active && problems && !calm;
  // SOTA виноват, только когда настройки прокси правда пропали.
  const sotaWiped = vpn.sota && env !== null && !env.points_to_bridge;
  if (!tunnel && !sotaWiped && !calm) return null;

  const copy = () => {
    void navigator.clipboard.writeText(hosts.join(", ")).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    });
  };
  const restoreEnv = async () => {
    setFixing(true);
    setFixError(null);
    try {
      setEnv(await setGlobalProxyEnv());
    } catch (e) {
      setFixError(String(e));
    } finally {
      setFixing(false);
    }
  };

  if (calm && !sotaWiped) {
    return (
      <p data-testid="vpn-calm" className={cn("flex items-center gap-2 px-1 text-[12px] text-vb-silver-dim", className)}>
        <ShieldCheck className="h-3.5 w-3.5 shrink-0 text-vb-emerald" />
        {bypass?.ok
          ? `Включён ${vpnName}. Мост ходит к прокси мимо него, всё работает.`
          : `Включён ${vpnName}. Мимо себя он не пускает, но прокси отвечают и через него.`}
      </p>
    );
  }

  return (
    <div
      data-testid="vpn-notice"
      className={cn("space-y-3 rounded-lg border border-vb-warn/30 bg-vb-warn/[0.05] px-4 py-3", className)}
    >
      {tunnel && (
        <div>
          <p className="text-[12px] font-medium text-vb-warn">
            {vpn.name ? `Включён VPN: ${vpn.name}` : "Включён VPN"}
          </p>
          <p className="mt-1 text-[12px] leading-snug text-vb-silver">
            {bypass?.ok === false
              ? "Он не пускает мост к прокси напрямую, поэтому прокси не отвечают. Выключите VPN или добавьте адрес прокси в исключения VPN."
              : "Мост ходит к прокси через него, поэтому прокси может не отвечать. Выключите VPN или добавьте адрес прокси в исключения VPN."}
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            <Button variant="secondary" size="sm" onClick={openVpnHelp}>
              Как добавить исключение
            </Button>
            {hosts.length > 0 && (
              <Button variant="ghost" size="sm" onClick={copy}>
                {copied ? <Check className="h-3.5 w-3.5 text-vb-emerald" /> : <Copy className="h-3.5 w-3.5" />}
                {copied ? "Скопировано" : hosts.length > 1 ? "Скопировать адреса" : "Скопировать адрес"}
              </Button>
            )}
          </div>
        </div>
      )}
      {sotaWiped && (
        <div>
          <p className="text-[12px] font-medium text-vb-warn">Пропали настройки прокси</p>
          <p className="mt-1 text-[12px] leading-snug text-vb-silver">
            Похоже, их стёр SOTA VPN. Терминал и Claude Code сейчас идут мимо моста. Верните настройки. Если они снова
            пропадут, закройте SOTA VPN или уберите его из автозагрузки.
          </p>
          <Button variant="secondary" size="sm" className="mt-2" onClick={restoreEnv} disabled={fixing}>
            {fixing && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            Вернуть настройки
          </Button>
          {fixError && <p className="mt-1.5 text-[11px] text-vb-loss">{fixError}</p>}
        </div>
      )}
    </div>
  );
}
