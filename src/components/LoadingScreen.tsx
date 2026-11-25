import { Component, useEffect, useRef, useState, type ReactNode } from "react";
import { BrandMark } from "./BrandMark";
import { Button } from "./Button";
import { reportCaughtUiError } from "../lib/uiWatchdog";
import { resetFailedViews } from "../lazyViews";

/** Не показывать «Проверяю…», если ответ пришёл быстрее. */
const SHOW_AFTER_MS = 150;
/** Показанную заглушку держим хотя бы столько: иначе она мелькнёт. */
const MIN_SHOWN_MS = 350;

/**
 * Спокойная заглушка загрузки: пока loading, первые 150 мс - ничего (быстрый
 * ответ без мигания), дальше - заглушка, и если уж она появилась, то
 * держится не меньше 350 мс. Возвращает true, пока надо показывать заглушку
 * (или пустой кадр до неё).
 */
export function useCalmLoading(loading: boolean): { waiting: boolean; shown: boolean } {
  const [shown, setShown] = useState(false);
  const [holding, setHolding] = useState(false);
  const shownAt = useRef<number | null>(null);
  useEffect(() => {
    if (loading) {
      if (shownAt.current !== null) return;
      const t = window.setTimeout(() => {
        shownAt.current = Date.now();
        setShown(true);
        setHolding(true);
      }, SHOW_AFTER_MS);
      return () => window.clearTimeout(t);
    }
    if (shownAt.current === null) return;
    const left = MIN_SHOWN_MS - (Date.now() - shownAt.current);
    const t = window.setTimeout(() => setHolding(false), Math.max(0, left));
    return () => window.clearTimeout(t);
  }, [loading]);
  return { waiting: loading || holding, shown };
}

interface Props {
  label?: string;
  /** Знак программы над подписью - для запуска окна. */
  brand?: boolean;
  /** Показать сразу (задержку уже выждал useCalmLoading). */
  immediate?: boolean;
}

/** Знак программы и короткая подпись по центру, мягкое появление. */
export function LoadingScreen({ label = "Проверяю…", brand = false, immediate = false }: Props) {
  const [shown, setShown] = useState(immediate);
  useEffect(() => {
    if (shown) return;
    const t = window.setTimeout(() => setShown(true), SHOW_AFTER_MS);
    return () => window.clearTimeout(t);
  }, [shown]);
  if (!shown) return <div className="h-full min-h-full w-full" aria-busy="true" />;
  return (
    <div
      role="status"
      className="view-enter flex h-full min-h-full w-full flex-col items-center justify-center gap-3"
      style={{ "--view-dir": 0 } as React.CSSProperties}
    >
      {brand && <BrandMark size={40} withStatus={false} />}
      <span className="text-[13px] text-vb-silver-dim">{label}</span>
    </div>
  );
}

/**
 * Раздел не загрузился (повреждённая установка, сбой в разделе): вместо
 * пустого окна - понятная строка и «Повторить». Остальное окно работает.
 */
export class PageBoundary extends Component<{ children: ReactNode; silent?: boolean }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidCatch(error: unknown) {
    reportCaughtUiError(error);
  }
  render() {
    if (!this.state.failed) return this.props.children;
    // Невидимая часть окна (окно обновления) - просто не показываем.
    if (this.props.silent) return null;
    return (
      <div role="alert" className="flex min-h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <span className="text-[13px] text-vb-silver">Раздел не открылся</span>
        <Button size="sm" variant="secondary" onClick={() => {
            resetFailedViews();
            this.setState({ failed: false });
          }}>
          Повторить
        </Button>
      </div>
    );
  }
}
