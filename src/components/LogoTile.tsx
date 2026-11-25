/** C-монограмма (как BrandMark, но без статуса - не нужен StatusProvider).
    Отдельно от UpdateDialog: её показывают уведомление и заставка обновления. */
export function LogoTile({ size }: { size: number }) {
  return (
    <svg viewBox="0 0 32 32" width={size} height={size} aria-hidden className="relative">
      <rect width="32" height="32" rx="7.5" fill="oklch(21% 0.012 160)" />
      <rect x="0.5" y="0.5" width="31" height="31" rx="7" fill="none" stroke="oklch(100% 0 0 / 0.08)" />
      <path
        d="M 23.09 21.54 A 9 9 0 1 1 23.09 10.46"
        fill="none"
        stroke="var(--color-vb-emerald)"
        strokeWidth="3.1"
        strokeLinecap="round"
      />
      <circle cx="25.2" cy="16" r="2.3" fill="var(--color-vb-emerald-bright)" />
    </svg>
  );
}
