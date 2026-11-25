import { useEffect, useRef, type RefObject } from "react";

const FOCUSABLE =
  "a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex='-1'])";

/*
  Фокус модального окна - как в ReportDialog / ShopClaimDialog: при открытии
  фокус на главную кнопку (initial), Tab не выходит из окна, Esc закрывает
  (если onEscape не null), после закрытия фокус возвращается туда, где был.
*/
export function useDialogFocus(
  open: boolean,
  dialogRef: RefObject<HTMLElement | null>,
  initialRef: RefObject<HTMLElement | null>,
  onEscape: (() => void) | null,
) {
  const escRef = useRef(onEscape);
  escRef.current = onEscape;

  useEffect(() => {
    if (!open) return;
    const before = document.activeElement as HTMLElement | null;
    // Окно появляется с анимацией - фокус чуть позже, когда кнопка уже есть.
    const t = window.setTimeout(() => {
      const el = initialRef.current ?? dialogRef.current?.querySelector<HTMLElement>(FOCUSABLE);
      el?.focus({ preventScroll: true });
    }, 60);
    const key = (e: KeyboardEvent) => {
      // Esc уже обработал кто-то внутри окна (подсказка, список) - окно не трогаем.
      if (e.defaultPrevented) return;
      if (e.key === "Escape" && escRef.current) {
        e.preventDefault();
        escRef.current();
        return;
      }
      if (e.key !== "Tab" || !dialogRef.current) return;
      const f = Array.from(dialogRef.current.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
        (el) => el.offsetParent !== null,
      );
      if (!f.length) return;
      const first = f[0];
      const last = f[f.length - 1];
      const inside = dialogRef.current.contains(document.activeElement);
      if (e.shiftKey && (document.activeElement === first || !inside)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (document.activeElement === last || !inside)) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", key);
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("keydown", key);
      // Вернуть фокус, если элемент ещё на странице (строку могли убрать).
      if (before && before.isConnected && before !== document.body) before.focus({ preventScroll: true });
    };
  }, [open, dialogRef, initialRef]);
}
