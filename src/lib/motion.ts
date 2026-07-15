import type { Transition, Variants } from "framer-motion";

/*
  Motion-пресеты по дизайн-скиллам (apple-design / impeccable / emil):
  - Springs с двумя параметрами (bounce + duration), НЕ физика mass/stiffness.
  - Дефолт bounce:0 (критич. демпфирование, без overshoot). Bounce только там,
    где жест нёс моментум — в этом приложении нигде, поэтому bounce везде 0.
  - Enter из opacity+translate(малый), НИКОГДА из scale(0).
  - Exit быстрее enter (~75%).
  - Только transform+opacity (GPU). Никаких bounce/elastic.
*/

/** Спринг для позиционирования/появления (дефолт UI). */
export const springBase: Transition = {
  type: "spring",
  bounce: 0,
  duration: 0.4,
};

/** Спринг снаппи — для мелких быстрых реакций (toggle, chip). */
export const springSnappy: Transition = {
  type: "spring",
  bounce: 0,
  duration: 0.28,
};

/** Твиновый переход для fade/цвета (ease-out-quint). */
export const easeQuint: Transition = {
  duration: 0.22,
  ease: [0.22, 1, 0.36, 1],
};

/** Быстрый твин для exit (75% от base). */
export const easeExit: Transition = {
  duration: 0.16,
  ease: [0.22, 1, 0.36, 1],
};

/** Появление карточки/секции снизу — enter из translateY, exit быстрее. */
export const fadeInUp: Variants = {
  hidden: { opacity: 0, y: 8 },
  show: {
    opacity: 1,
    y: 0,
    transition: springBase,
  },
  exit: {
    opacity: 0,
    y: 4,
    transition: easeExit,
  },
};

/** Появление без сдвига (для оверлеев/результатов). */
export const fadeIn: Variants = {
  hidden: { opacity: 0 },
  show: { opacity: 1, transition: easeQuint },
  exit: { opacity: 0, transition: easeExit },
};

/** Контейнер со stagger детей (30–80ms между элементами). */
export const staggerContainer: Variants = {
  hidden: {},
  show: {
    transition: { staggerChildren: 0.05, delayChildren: 0.04 },
  },
};

/** Press-feedback пресет для интерактивных элементов (scale 0.97). */
export const pressable = {
  whileTap: { scale: 0.97 },
  transition: { duration: 0.12, ease: [0.22, 1, 0.36, 1] as const },
};
