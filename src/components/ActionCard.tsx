import { type ReactNode } from "react";
import { ChevronRight } from "lucide-react";

interface Props {
  icon: ReactNode;
  title: string;
  description: string;
  onClick: () => void;
}

export function ActionCard({ icon, title, description, onClick }: Props) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group flex w-full items-center gap-4 rounded-[10px] border border-white/[0.04] bg-[rgba(10,10,10,0.55)] px-6 py-5 text-left backdrop-blur-2xl transition-all duration-200 hover:border-white/[0.08] hover:bg-[rgba(16,185,129,0.04)]"
    >
      <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-white/[0.04] bg-white/[0.03] text-vb-silver">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-[15px] font-medium tracking-tight text-vb-silver">
          {title}
        </div>
        <div className="mt-1 text-[13px] leading-relaxed text-vb-silver-dim">
          {description}
        </div>
      </div>
      <ChevronRight className="h-4 w-4 shrink-0 text-vb-silver-dim/60 transition-all group-hover:translate-x-0.5 group-hover:text-vb-emerald" />
    </button>
  );
}
