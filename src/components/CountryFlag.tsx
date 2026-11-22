interface Props {
  code: string;
}

/*
  Флаги лежат в программе (src/assets/flags, 48×36 с flagcdn.com), а не
  грузятся с сайта: у части людей он недоступен, и вместо флага было
  значок битой картинки. Неизвестная страна - без флага.
*/
const FLAGS = import.meta.glob<string>("../assets/flags/*.png", {
  eager: true,
  import: "default",
  query: "?url",
});

function flagUrl(code: string): string | undefined {
  return FLAGS[`../assets/flags/${code.toLowerCase()}.png`];
}

/*
  Флаг страны - в размер текста рядом: высота ≈ высоте заглавных букв
  (0.8em), пропорции 4:3. Тонкая обводка - чтобы светлые флаги
  (Финляндия, Япония) не сливались с фоном.
*/
export function CountryFlag({ code }: Props) {
  const src = flagUrl(code);
  if (!src) return null;
  return (
    <img
      src={src}
      alt={code.toUpperCase()}
      draggable={false}
      className="inline-block shrink-0 rounded-[2px] object-cover outline outline-1 -outline-offset-1 outline-white/10"
      style={{ width: "1.07em", height: "0.8em" }}
    />
  );
}
