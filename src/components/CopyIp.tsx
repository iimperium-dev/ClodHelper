import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "../lib/cn";

/** IP прокси: при наведении - значок копирования, по клику - в буфер обмена. */
export function CopyIp({ text, copy, className }: { text: string; copy: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(copy);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* буфер обмена недоступен - молча */
    }
  };
  return (
    <button
      type="button"
      onClick={(e) => {
        // Внутри кликабельной строки (выбор прокси в настройках приложения)
        // копирование не должно выбирать строку.
        e.stopPropagation();
        void onCopy();
      }}
      title={copied ? "Скопировано" : "Скопировать IP"}
      className={cn(
        "group/ip inline-flex min-w-0 items-center rounded font-mono transition-colors hover:text-vb-silver",
        className,
      )}
    >
      <span className="truncate">{text}</span>
      {/* Значок в покое места не занимает - иначе после IP висел пустой
          отступ («72.56.129.22   · 58 мс»); раскрывается при наведении. */}
      {copied ? (
        <Check className="ml-1 h-3 w-3 shrink-0 text-vb-emerald" />
      ) : (
        <Copy className="ml-0 h-3 w-0 shrink-0 opacity-0 transition-all group-hover/ip:ml-1 group-hover/ip:w-3 group-hover/ip:opacity-100 group-focus-visible/ip:ml-1 group-focus-visible/ip:w-3 group-focus-visible/ip:opacity-100" />
      )}
    </button>
  );
}
