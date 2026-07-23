import { useEffect, useRef, useState } from "react";
import { animate, useReducedMotion } from "framer-motion";

interface Props {
  value: number;
  className?: string;
}

/*
  Число, докручивающееся до нового значения (number ticker). Обязательно
  tabular-nums, чтобы цифры не прыгали по ширине (правило скиллов).
*/
export function NumberTicker({ value, className }: Props) {
  const reduce = useReducedMotion();
  const [display, setDisplay] = useState(value);
  const prev = useRef(value);

  useEffect(() => {
    if (prev.current === value) return;
    if (reduce) {
      prev.current = value;
      setDisplay(value);
      return;
    }
    const controls = animate(prev.current, value, {
      duration: 0.6,
      ease: [0.22, 1, 0.36, 1],
      onUpdate: (v) => setDisplay(Math.round(v)),
    });
    prev.current = value;
    return () => controls.stop();
  }, [value, reduce]);

  return <span className={`tnum ${className ?? ""}`}>{display.toLocaleString("ru-RU")}</span>;
}
