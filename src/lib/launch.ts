/*
  Claude Desktop открыт без прокси, а человек нажал «Открыть». Это не сбой:
  бэкенд отвечает этим текстом (claude_desktop::RESTART_OFFER, текст тот же),
  а интерфейс показывает его с кнопкой «Перезапустить». Ошибкой не
  считается и в отчёты не уходит.
*/
export const CLAUDE_RESTART_OFFER = "Claude Desktop открыт без прокси. Перезапустить его с прокси?";

export const isRestartOffer = (e: unknown): boolean => String(e) === CLAUDE_RESTART_OFFER;
