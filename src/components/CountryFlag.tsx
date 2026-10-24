interface Props {
  code: string;
}

/*
  Флаг страны — в размер текста рядом: высота ≈ высоте заглавных букв
  (0.8em), пропорции 4:3. Фиксированные 16×12 px рядом с подписью 11 px
  выглядели крупнее текста. Тонкая обводка — чтобы светлые флаги
  (Финляндия, Япония) не сливались с фоном.
*/
export function CountryFlag({ code }: Props) {
  const low = code.toLowerCase();
  return (
    <img
      src={`https://flagcdn.com/32x24/${low}.png`}
      srcSet={`https://flagcdn.com/48x36/${low}.png 2x, https://flagcdn.com/64x48/${low}.png 3x`}
      alt={code.toUpperCase()}
      draggable={false}
      className="inline-block shrink-0 rounded-[2px] object-cover outline outline-1 -outline-offset-1 outline-white/10"
      style={{ width: "1.07em", height: "0.8em" }}
    />
  );
}
