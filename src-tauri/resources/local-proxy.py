#!/usr/bin/env python3
"""
local-proxy.py — HTTP-bridge: слушает 127.0.0.1:8888 без auth и форвардит
трафик в купленный HTTP-прокси с auth (login:pass@ip:port).

Зачем: Electron-приложения (Claude Desktop, Cursor, VS Code) при запуске
с флагом --proxy-server=http://127.0.0.1:8888 не показывают auth-popup
и работают прозрачно. Не трогает остальную систему.

Использует только встроенные модули Python 3.7+ (socket, threading, base64).
Никаких pip install.

Запуск:
    python local-proxy.py --upstream http://login:pass@1.2.3.4:8000

Опции:
    --upstream URL       URL купленного прокси (обязательно)
    --listen HOST:PORT   На каком интерфейсе слушать (по умолчанию 127.0.0.1:8888)
"""

from __future__ import annotations

import argparse
import base64
import logging
import socket
import sys
import threading
import time
from urllib.parse import urlparse

# Потолок одновременных проброшенных соединений. Защита от пика деплоя
# (ssh/scp/docker-pull открывают шквал соединений через шим) — без лимита
# процесс плодил по потоку на коннект → too-many-threads/OOM → шим падал,
# рвя SSE-стрим Claude Code. Семафор держит число живых хендлеров; лишние
# ждут слота, а не валят процесс. 256 с запасом для нормального стрима+деплоя.
_MAX_CONNECTIONS = 256
_conn_slots = threading.Semaphore(_MAX_CONNECTIONS)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(message)s",
    datefmt="%H:%M:%S",
)
log = logging.getLogger("local-proxy")


def parse_upstream(url: str) -> tuple[str, int, str | None]:
    """http://login:pass@ip:port → (host, port, basic_auth_b64 | None)."""
    p = urlparse(url)
    if p.scheme not in ("http", "https"):
        raise ValueError(f"Upstream scheme must be http/https, got: {p.scheme}")
    if not p.hostname or not p.port:
        raise ValueError(f"Upstream URL must include host:port, got: {url}")
    auth: str | None = None
    if p.username and p.password:
        creds = f"{p.username}:{p.password}".encode()
        auth = base64.b64encode(creds).decode()
    return p.hostname, p.port, auth


def enable_keepalive(sock: socket.socket) -> None:
    """Включить TCP keepalive: NAT/firewall между нами и upstream не должен
    тихо убивать idle-соединение посреди долгого SSE-стрима Claude Code.

    Идея: каждые 30 сек слать невидимый probe-пакет (заголовок TCP без
    данных). Если 5 проб подряд не получили ACK — рвём соединение явно
    (а не молча получаем RST через несколько минут).
    """
    try:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_KEEPALIVE, 1)
    except OSError:
        return
    # Платформо-специфичные опции — на Windows работают через TCP_KEEPIDLE
    # с Python 3.7+. На Linux — те же константы. На macOS — TCP_KEEPALIVE.
    try:
        if hasattr(socket, "TCP_KEEPIDLE"):
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPIDLE, 30)
        elif hasattr(socket, "TCP_KEEPALIVE"):
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPALIVE, 30)
        if hasattr(socket, "TCP_KEEPINTVL"):
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPINTVL, 10)
        if hasattr(socket, "TCP_KEEPCNT"):
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_KEEPCNT, 5)
    except OSError:
        pass


def pipe(src: socket.socket, dst: socket.socket) -> None:
    """Перекачивать байты src → dst пока кто-то из них не закроется."""
    try:
        while True:
            data = src.recv(8192)
            if not data:
                break
            dst.sendall(data)
    except (OSError, ConnectionError):
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


def handle_client(
    client: socket.socket,
    addr: tuple[str, int],
    upstream_host: str,
    upstream_port: int,
    upstream_auth: str | None,
) -> None:
    """Один client-коннект: пробрасываем первый CONNECT/request и потом 2-way pipe."""
    try:
        client.settimeout(30)
        # Читаем первый chunk запроса — там headers.
        head_buf = b""
        while b"\r\n\r\n" not in head_buf:
            chunk = client.recv(8192)
            if not chunk:
                return
            head_buf += chunk
            if len(head_buf) > 65536:
                log.warning("Headers too large from %s, dropping", addr)
                return

        # Подключаемся к upstream-прокси.
        upstream = socket.create_connection((upstream_host, upstream_port), timeout=30)

        # Включаем TCP keepalive на оба сокета — Claude Code держит SSE-стрим
        # минутами; без keepalive NAT/прокси-провайдер тихо убивают idle
        # соединение → клиент получает "socket closed unexpectedly".
        enable_keepalive(client)
        enable_keepalive(upstream)

        # Снимаем 30-сек timeout с client после прокидки заголовков —
        # дальше идёт long-lived стрим, блокирующий recv() — это норма.
        client.settimeout(None)
        upstream.settimeout(None)

        # Если есть auth — нужно вставить Proxy-Authorization в первый запрос.
        # CONNECT-запрос и обычный HTTP-запрос обрабатываем одинаково: просто
        # добавляем header перед \r\n\r\n.
        if upstream_auth:
            header_end = head_buf.index(b"\r\n\r\n")
            head_lines = head_buf[:header_end]
            body = head_buf[header_end:]
            # Удалить существующий Proxy-Authorization если есть (на всякий случай)
            new_lines = [
                ln for ln in head_lines.split(b"\r\n")
                if not ln.lower().startswith(b"proxy-authorization:")
            ]
            new_lines.append(f"Proxy-Authorization: Basic {upstream_auth}".encode())
            head_buf = b"\r\n".join(new_lines) + body

        upstream.sendall(head_buf)

        # Дальше — простой 2-way pipe в отдельных потоках.
        t1 = threading.Thread(target=pipe, args=(client, upstream), daemon=True)
        t2 = threading.Thread(target=pipe, args=(upstream, client), daemon=True)
        t1.start()
        t2.start()
        t1.join()
        t2.join()
    except (OSError, ConnectionError, socket.timeout) as e:
        log.debug("Conn error from %s: %s", addr, e)
    finally:
        try:
            client.close()
        except OSError:
            pass
        _conn_slots.release()  # вернуть слот семафора (захвачен в accept-цикле)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Local HTTP-proxy bridge (no-auth localhost → auth upstream)",
    )
    parser.add_argument(
        "--upstream",
        required=True,
        help="Upstream proxy URL: http://login:pass@ip:port",
    )
    parser.add_argument(
        "--listen",
        default="127.0.0.1:8888",
        help="Bind address (default: 127.0.0.1:8888)",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="Less logging",
    )
    args = parser.parse_args()

    if args.quiet:
        log.setLevel(logging.WARNING)

    try:
        upstream_host, upstream_port, upstream_auth = parse_upstream(args.upstream)
    except ValueError as e:
        log.error("Invalid --upstream: %s", e)
        return 1

    listen_host, listen_port_str = args.listen.rsplit(":", 1)
    listen_port = int(listen_port_str)

    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        server.bind((listen_host, listen_port))
    except OSError as e:
        log.error("Cannot bind %s:%s — %s", listen_host, listen_port, e)
        log.error("Is another process already listening on this port?")
        return 1
    # Backlog побольше: при пике деплоя десятки соединений встают в очередь
    # ОС — без запаса часть отбивается RST, и Claude Code видит обрыв.
    server.listen(256)

    log.info(
        "Listening on %s:%s → upstream %s:%s%s",
        listen_host,
        listen_port,
        upstream_host,
        upstream_port,
        " (with auth)" if upstream_auth else "",
    )
    log.info("Use proxy URL: http://%s:%s in apps", listen_host, listen_port)

    # ГЛАВНЫЙ accept-цикл НИКОГДА не должен падать (иначе слушающий сокет
    # исчезает → ConnectionRefused, Claude Code теряет связь, деплой гаснет).
    # Любую ошибку accept/запуска потока ловим и продолжаем, а не выходим.
    try:
        while True:
            try:
                client, addr = server.accept()
            except OSError as e:
                # transient (EMFILE «too many open files» под пиком деплоя,
                # сетевой сбой) — НЕ валим сервер, ждём чуть и продолжаем.
                log.warning("accept() failed (%s) — продолжаю слушать", e)
                time.sleep(0.1)
                continue

            # лимит одновременных соединений: ждём свободный слот (с таймаутом,
            # чтобы зависший пик не заблокировал приём навсегда). Нет слота за
            # 5с → отбиваем этот коннект, но СЕРВЕР живёт (приоритет — не упасть).
            if not _conn_slots.acquire(timeout=5.0):
                log.warning("Лимит %d соединений — отбиваю %s, сервер жив",
                            _MAX_CONNECTIONS, addr)
                try:
                    client.close()
                except OSError:
                    pass
                continue

            try:
                t = threading.Thread(
                    target=handle_client,
                    args=(client, addr, upstream_host, upstream_port, upstream_auth),
                    daemon=True,
                )
                t.start()
            except (RuntimeError, OSError) as e:
                # can't start new thread — освобождаем слот, закрываем коннект,
                # но сервер ПРОДОЛЖАЕТ слушать (не падаем).
                log.warning("Не смог запустить поток (%s) — отбиваю %s", e, addr)
                _conn_slots.release()
                try:
                    client.close()
                except OSError:
                    pass
    except KeyboardInterrupt:
        log.info("Stopping (Ctrl+C)")
        return 0
    finally:
        server.close()


if __name__ == "__main__":
    sys.exit(main())
