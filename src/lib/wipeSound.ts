/*
  Звук «очистка готова» — «Смахнуло»: мягкий взмах (шум, полоса фильтра
  скользит вверх, будто сдули пыль) и тихий «дзинь» в конце. Синтез
  WebAudio, без файлов. Выбран владельцем из пяти вариантов.

  Играет WebView: звук идёт сразу после нажатия «Да, очистить», поэтому
  аудиоконтекст создаём в момент нажатия (unlockWipeSound) — жест
  пользователя разблокирует воспроизведение.
*/

let ctx: AudioContext | null = null;

/** Создать/разбудить аудиоконтекст в момент нажатия (жест пользователя). */
export function unlockWipeSound(): void {
  try {
    ctx ??= new AudioContext();
    void ctx.resume();
  } catch {
    /* без звука — не страшно */
  }
}

function ding(c: AudioContext, out: AudioNode, f: number, at: number, vol: number, decay: number) {
  const t0 = c.currentTime + at;
  const o = c.createOscillator();
  const g = c.createGain();
  o.type = "sine";
  o.frequency.value = f;
  g.gain.setValueAtTime(0, t0);
  g.gain.linearRampToValueAtTime(vol, t0 + 0.005);
  g.gain.setTargetAtTime(0, t0 + 0.005, 1 / decay);
  o.connect(g).connect(out);
  o.start(t0);
  o.stop(t0 + 1.2);
}

/** Сыграть «Смахнуло». */
export function playWipeSound(): void {
  try {
    ctx ??= new AudioContext();
    void ctx.resume();
    const c = ctx;
    const out = c.createGain();
    out.gain.value = 0.9;
    out.connect(c.destination);

    // Взмах: белый шум через полосовой фильтр, частота скользит 500 → 5200 Гц.
    const buf = c.createBuffer(1, Math.floor(c.sampleRate * 0.6), c.sampleRate);
    const d = buf.getChannelData(0);
    for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
    const n = c.createBufferSource();
    n.buffer = buf;
    const bp = c.createBiquadFilter();
    bp.type = "bandpass";
    bp.Q.value = 1.4;
    const g = c.createGain();
    const t0 = c.currentTime;
    bp.frequency.setValueAtTime(500, t0);
    bp.frequency.exponentialRampToValueAtTime(5200, t0 + 0.42);
    g.gain.setValueAtTime(0, t0);
    g.gain.linearRampToValueAtTime(0.32, t0 + 0.18);
    g.gain.linearRampToValueAtTime(0, t0 + 0.5);
    n.connect(bp).connect(g).connect(out);
    n.start(t0);
    n.stop(t0 + 0.6);

    // «Дзинь» в конце взмаха: ля6 и тихий призвук.
    ding(c, out, 1760, 0.42, 0.14, 4);
    ding(c, out, 2637, 0.42, 0.04, 9);
  } catch {
    /* без звука — не страшно */
  }
}
