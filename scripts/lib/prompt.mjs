// Ввод пароля со скрытым эхом (без зависимостей).

export function askHidden(prompt) {
  return new Promise((resolve, reject) => {
    const stdin = process.stdin;
    if (!stdin.isTTY) {
      reject(new Error("нужен интерактивный терминал (или переменная CLOD_SIGN_PASS)"));
      return;
    }
    process.stdout.write(prompt);
    let buf = "";
    const finish = (fn, v) => {
      stdin.off("data", onData);
      stdin.setRawMode(false);
      stdin.pause();
      process.stdout.write("\n");
      fn(v);
    };
    const onData = (chunk) => {
      for (const c of String(chunk)) {
        if (c === "\r" || c === "\n") return finish(resolve, buf);
        if (c === "\u0003") return finish(reject, new Error("отменено"));
        if (c === "\u007f" || c === "\b") buf = buf.slice(0, -1);
        else if (c >= " ") buf += c;
      }
    };
    stdin.setRawMode(true);
    stdin.resume();
    stdin.setEncoding("utf8");
    stdin.on("data", onData);
  });
}
