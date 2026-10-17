#!/usr/bin/env python3
"""Иконки строки меню macOS для Clod Helper (без внешних зависимостей).

  tray-mac-template.png      — норма: чёрный + альфа, isTemplate (macOS сама
                               красит под светлую/тёмную строку меню);
  tray-mac-{warn,down}-{light,dark}.png — тревога: «C» в цвет текста строки
                               меню (light — тёмная «C» для светлой строки,
                               dark — светлая для тёмной) + цветная точка.

Цветную точку нельзя держать в template-картинке: macOS перекрашивает её
целиком. Поэтому тревожные состояния — обычные картинки в двух вариантах.

Запуск: python3 scripts/gen-tray-icons-mac.py (пишет в src-tauri/icons).
"""
import math
import os
import struct
import zlib

SIZE = 64          # как у Windows-иконок трея; tray-icon масштабирует под 18pt
SS = 4             # суперсэмплинг для сглаживания
CX, CY = 29.0, 29.0
R_OUT, R_IN = 22.0, 14.5
GAP_DEG = 42.0     # полураскрытие «C» вправо
DOT = (47.0, 47.0)
DOT_R_NODE = 6.5   # маленький «узел» логотипа в норме
DOT_R_ALERT = 10.5 # точка статуса при тревоге
CUTOUT = 3.0       # прозрачный зазор вокруг точки

OUT = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")


def coverage(fn):
    """Доля субпикселей пикселя (x, y), где fn истинна."""
    grid = [[0.0] * SIZE for _ in range(SIZE)]
    step = 1.0 / SS
    for y in range(SIZE):
        for x in range(SIZE):
            hits = 0
            for sy in range(SS):
                for sx in range(SS):
                    if fn(x + (sx + 0.5) * step, y + (sy + 0.5) * step):
                        hits += 1
            grid[y][x] = hits / (SS * SS)
    return grid


def in_c(px, py, dot_r):
    dx, dy = px - CX, py - CY
    r = math.hypot(dx, dy)
    if not (R_IN <= r <= R_OUT):
        return False
    ang = math.degrees(math.atan2(dy, dx))
    if abs(ang) < GAP_DEG:
        return False
    # Вырез вокруг точки — точка читается как «бейдж» поверх «C».
    return math.hypot(px - DOT[0], py - DOT[1]) > dot_r + CUTOUT


def in_dot(px, py, dot_r):
    return math.hypot(px - DOT[0], py - DOT[1]) <= dot_r


def write_png(path, pixels):
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in pixels)
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def compose(layers):
    """layers: [(coverage, (r, g, b, a))] снизу вверх → RGBA-пиксели."""
    out = []
    for y in range(SIZE):
        row = []
        for x in range(SIZE):
            r = g = b = a = 0.0
            for cov, (cr, cg, cb, ca) in layers:
                sa = cov[y][x] * ca
                if sa <= 0:
                    continue
                na = sa + a * (1 - sa)
                r = (cr * sa + r * a * (1 - sa)) / na
                g = (cg * sa + g * a * (1 - sa)) / na
                b = (cb * sa + b * a * (1 - sa)) / na
                a = na
            row.append((round(r), round(g), round(b), round(a * 255)))
        out.append(row)
    return out


def main():
    os.makedirs(OUT, exist_ok=True)
    black = (0, 0, 0, 1.0)
    c_node = coverage(lambda x, y: in_c(x, y, DOT_R_NODE))
    node = coverage(lambda x, y: in_dot(x, y, DOT_R_NODE))
    write_png(os.path.join(OUT, "tray-mac-template.png"), compose([(c_node, black), (node, black)]))

    c_alert = coverage(lambda x, y: in_c(x, y, DOT_R_ALERT))
    dot = coverage(lambda x, y: in_dot(x, y, DOT_R_ALERT))
    ink = {"light": (0, 0, 0, 0.85), "dark": (255, 255, 255, 0.92)}
    colors = {"warn": (233, 162, 59, 1.0), "down": (229, 72, 77, 1.0)}
    for state, color in colors.items():
        for bar, c_color in ink.items():
            path = os.path.join(OUT, f"tray-mac-{state}-{bar}.png")
            write_png(path, compose([(c_alert, c_color), (dot, color)]))
    print("ok:", OUT)


if __name__ == "__main__":
    main()
