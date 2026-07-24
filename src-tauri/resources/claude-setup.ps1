#Requires -Version 5.1
<#
.SYNOPSIS
    Установка Claude Code с прокси для региона где Anthropic API недоступен напрямую.

.DESCRIPTION
    Делает за пользователя 5 шагов:
      1. Ставит Node.js LTS если его нет.
      2. Ставит Claude Code CLI глобально через npm.
      3. Прописывает HTTPS_PROXY / HTTP_PROXY в системные переменные пользователя.
      4. Проверяет что Anthropic API отвечает через прокси.
      5. Создаёт ярлык на рабочем столе для быстрого запуска.

    После установки команда `claude` доступна в любом терминале (PowerShell / cmd / VS Code).

.PARAMETER ProxyUrl
    URL прокси в формате http://login:pass@ip:port. Если не задан — спросит интерактивно.

.PARAMETER SkipNodeInstall
    Не пытаться ставить Node.js (если уверен что он есть).

.PARAMETER Uninstall
    Удалить Claude Code и сбросить proxy env-vars. Прокси при этом удаляется только
    из user-scope, system-wide остаётся как есть.

.EXAMPLE
    .\claude-setup.ps1
    Интерактивный режим — спросит прокси.

.EXAMPLE
    .\claude-setup.ps1 -ProxyUrl "http://login:pass@1.2.3.4:8000"
    Без интерактивных вопросов.

.EXAMPLE
    .\claude-setup.ps1 -Uninstall
    Удалить Claude Code.

.NOTES
    Версия: 1.0
    Автор: iimperium tools
    Тестировано: Windows 10/11, PowerShell 5.1+
#>

param(
    [string]$ProxyUrl,
    [switch]$SkipNodeInstall,
    [switch]$Uninstall,
    [switch]$Yes  # non-interactive: пропускает confirm-prompt
)

$ErrorActionPreference = 'Stop'

# ─── Цветовые helpers ─────────────────────────────────────────────

function Write-Header {
    param([string]$Text)
    Write-Host ""
    Write-Host "┌─────────────────────────────────────────────────┐" -ForegroundColor Cyan
    $padded = $Text.PadRight(47)
    Write-Host "│ $padded │" -ForegroundColor Cyan
    Write-Host "└─────────────────────────────────────────────────┘" -ForegroundColor Cyan
    Write-Host ""
}

function Write-Step {
    param([int]$N, [int]$Total, [string]$Text)
    Write-Host ("  [{0}/{1}] " -f $N, $Total) -NoNewline -ForegroundColor DarkGray
    Write-Host $Text
}

function Write-Ok {
    param([string]$Text = "OK")
    Write-Host "        ✓ $Text" -ForegroundColor Green
}

function Write-Warn {
    param([string]$Text)
    Write-Host "        ⚠ $Text" -ForegroundColor Yellow
}

function Write-Err {
    param([string]$Text)
    Write-Host "        ✗ $Text" -ForegroundColor Red
}

function Read-WithDefault {
    param([string]$Prompt, [string]$Default = "")
    if ($Default) {
        $input = Read-Host "$Prompt [по умолчанию: $Default]"
        if ([string]::IsNullOrWhiteSpace($input)) { return $Default }
        return $input
    }
    return Read-Host $Prompt
}

# ─── Validation helpers ───────────────────────────────────────────

function Test-ProxyUrl {
    param([string]$Url)
    # http://login:pass@ip:port — простой регексп.
    return $Url -match '^https?://[^:@]+:[^@]+@[\d\.\w\-]+:\d+/?$'
}

function Test-NodeInstalled {
    try {
        $v = & node --version 2>&1
        if ($LASTEXITCODE -eq 0 -and $v -match '^v(\d+)\.') {
            return [int]$matches[1]  # major-версия
        }
    } catch {
        return 0
    }
    return 0
}

function Test-ClaudeInstalled {
    try {
        $v = & claude --version 2>&1
        if ($LASTEXITCODE -eq 0) {
            return $v
        }
    } catch {
        return $null
    }
    return $null
}

# ─── Uninstall mode ───────────────────────────────────────────────

if ($Uninstall) {
    Write-Header "Удаление Claude Code"

    Write-Step 1 3 "Удаляю Claude Code через npm..."
    try {
        & npm uninstall -g @anthropic-ai/claude-code 2>&1 | Out-Null
        Write-Ok "удалён"
    } catch {
        Write-Warn "не удалось — возможно уже удалён"
    }

    Write-Step 2 3 "Сбрасываю HTTPS_PROXY / HTTP_PROXY..."
    [Environment]::SetEnvironmentVariable("HTTPS_PROXY", $null, "User")
    [Environment]::SetEnvironmentVariable("HTTP_PROXY", $null, "User")
    Write-Ok "сброшены"

    Write-Step 3 3 "Удаляю ярлык с рабочего стола..."
    $desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Claude Code.lnk'
    if (Test-Path $desktopShortcut) {
        Remove-Item $desktopShortcut -Force
        Write-Ok "удалён"
    } else {
        Write-Ok "не найден (это нормально)"
    }

    Write-Host ""
    Write-Host "Готово. Claude Code удалён." -ForegroundColor Green
    Write-Host "Node.js не трогали — если хочешь удалить, через 'Установка и удаление программ'." -ForegroundColor DarkGray
    exit 0
}

# ─── Install mode ─────────────────────────────────────────────────

Write-Header "Claude Code Setup v1.0"
Write-Host "  Этот скрипт настроит Claude Code с прокси для Anthropic API."
Write-Host "  Займёт 5-10 минут. Безопасно прервать Ctrl+C на любом шаге."
Write-Host ""

# Шаг 0 — получить прокси
if (-not $ProxyUrl) {
    Write-Host "  Введи URL купленного HTTP-прокси:" -ForegroundColor Yellow
    Write-Host "    Формат: http://login:pass@ip:port" -ForegroundColor DarkGray
    Write-Host "    Пример: http://user123:Pa55w0rd@45.130.61.5:8000" -ForegroundColor DarkGray
    Write-Host ""
    $ProxyUrl = Read-Host "  Прокси"
}

if (-not (Test-ProxyUrl $ProxyUrl)) {
    Write-Err "Прокси не похож на правильный URL."
    Write-Host "  Ожидаемый формат: http://login:pass@ip:port" -ForegroundColor Yellow
    Write-Host "  Получено: $ProxyUrl" -ForegroundColor DarkGray
    exit 1
}

Write-Host ""
Write-Host "  Прокси: $ProxyUrl" -ForegroundColor DarkGray
if (-not $Yes) {
    $confirm = Read-Host "  Всё верно? Продолжить установку? (y/n)"
    if ($confirm -notmatch '^[yYдД]') {
        Write-Host "  Отменено." -ForegroundColor Yellow
        exit 0
    }
}

# ─── Шаг 1: Node.js ───────────────────────────────────────────────

Write-Host ""
Write-Step 1 5 "Проверяю Node.js..."

$nodeMajor = Test-NodeInstalled
if ($nodeMajor -ge 18) {
    Write-Ok "найден v$nodeMajor"
} elseif ($nodeMajor -gt 0) {
    Write-Warn "найден старый Node.js v$nodeMajor (нужна ≥18)"
    if (-not $SkipNodeInstall) {
        Write-Host "        Скачай LTS вручную с https://nodejs.org и перезапусти этот скрипт." -ForegroundColor Yellow
        exit 1
    }
} else {
    if ($SkipNodeInstall) {
        Write-Err "Node.js не найден, а -SkipNodeInstall задан."
        exit 1
    }
    Write-Warn "не найден — открываю страницу скачивания..."
    Write-Host "        Скачай Windows Installer (.msi) с https://nodejs.org" -ForegroundColor Yellow
    Write-Host "        После установки перезапусти этот скрипт." -ForegroundColor Yellow
    Start-Process "https://nodejs.org/en/download"
    exit 1
}

# ─── Шаг 2: Claude Code CLI ──────────────────────────────────────

Write-Step 2 5 "Устанавливаю Claude Code..."

$existingClaude = Test-ClaudeInstalled
if ($existingClaude) {
    Write-Ok "уже установлен ($existingClaude)"
    Write-Host "        Обновляю до последней версии..." -ForegroundColor DarkGray
}

# Временно выставляем прокси для npm (чтобы скачивание шло через него,
# если у юзера и npm-registry заблокирован).
$env:HTTPS_PROXY = $ProxyUrl
$env:HTTP_PROXY = $ProxyUrl

Write-Host "        Скачиваю с npm-registry (~30-90 секунд)..." -ForegroundColor DarkGray

# Запускаем npm в фоне с захватом stdout/stderr в файлы. Не используем 2>&1
# в pipe — npm с --loglevel=http пишет служебные строки в stderr, и под
# $ErrorActionPreference='Continue' (наша обёртка) ConvertFrom-Json трактует
# их как exceptions, что роняет catch несмотря на успешный exit-code.
$npmOut = [System.IO.Path]::GetTempFileName()
$npmErr = [System.IO.Path]::GetTempFileName()
try {
    $proc = Start-Process -FilePath "npm.cmd" `
        -ArgumentList @("install", "-g", "@anthropic-ai/claude-code", "--loglevel=http") `
        -NoNewWindow -PassThru -Wait `
        -RedirectStandardOutput $npmOut `
        -RedirectStandardError $npmErr
    $exitCode = $proc.ExitCode

    # Парсим оба лога чтобы выдать сокращённые маркеры пользователю.
    $allLines = @()
    if (Test-Path $npmOut) { $allLines += Get-Content $npmOut -ErrorAction SilentlyContinue }
    if (Test-Path $npmErr) { $allLines += Get-Content $npmErr -ErrorAction SilentlyContinue }
    foreach ($line in $allLines) {
        $line = $line.Trim()
        if (-not $line) { continue }
        if ($line -match '^npm http fetch GET 200 https?://[^/]+/([^\?]+)') {
            Write-Host "        ↓ скачиваю $($matches[1])" -ForegroundColor DarkGray
        } elseif ($line -match '^added \d+ packages?') {
            Write-Host "        ✓ $line" -ForegroundColor Green
        } elseif ($line -match '^changed \d+ packages?') {
            Write-Host "        ✓ $line" -ForegroundColor Green
        } elseif ($line -match '^up to date') {
            Write-Host "        ✓ уже последняя версия" -ForegroundColor Green
        } elseif ($line -match '^npm (err|ERR)') {
            Write-Host "        $line" -ForegroundColor Red
        }
        # Остальное пропускаем — http GET 304, fetch start, warn, etc.
    }

    if ($exitCode -ne 0) {
        throw "npm exit code $exitCode (см. $npmErr)"
    }
    Write-Ok "установлен"
} catch {
    Write-Err "не удалось установить через npm"
    Write-Host "        Ошибка: $_" -ForegroundColor DarkGray
    Write-Host "        Попробуй вручную: npm install -g @anthropic-ai/claude-code" -ForegroundColor Yellow
    exit 1
} finally {
    Remove-Item $npmOut -Force -ErrorAction SilentlyContinue
    Remove-Item $npmErr -Force -ErrorAction SilentlyContinue
}

# ─── Шаг 3: системные env-vars → локальный bridge ────────────────

Write-Step 3 5 "Прописываю HTTPS_PROXY → локальный bridge (127.0.0.1:8889)..."

# Указываем на ЛОКАЛЬНЫЙ bridge, а не на upstream прокси напрямую.
# Это решает две проблемы:
#  - Electron-приложения (вроде Claude Desktop) ломаются на auth в URL
#    прокси-сервера. С локальным bridge без auth они работают штатно.
#  - Не плодим разные прокси-цепочки для CLI и Desktop: всё ходит через
#    один мост, который инжектит auth и форвардит в upstream.
$BridgeUrl = "http://127.0.0.1:8889"

try {
    [Environment]::SetEnvironmentVariable("HTTPS_PROXY", $BridgeUrl, "User")
    [Environment]::SetEnvironmentVariable("HTTP_PROXY", $BridgeUrl, "User")
    # Применяем в текущем процессе чтобы шаг 4 сработал.
    $env:HTTPS_PROXY = $BridgeUrl
    $env:HTTP_PROXY = $BridgeUrl
    Write-Ok "переменные окружения выставлены ($BridgeUrl)"
} catch {
    Write-Err "не удалось записать env-vars"
    Write-Host "        Ошибка: $_" -ForegroundColor DarkGray
    exit 1
}

# ─── Шаг 4: проверка коннекта к Anthropic ────────────────────────

Write-Step 4 5 "Проверяю что Anthropic API доступен через прокси..."

try {
    # /v1/messages без auth даст 401 — это означает что прокси работает,
    # и API нас видит. ERR_TIMED_OUT / 502 — значит прокси мёртв или плохой.
    # -Proxy ОБЯЗАТЕЛЕН: Invoke-WebRequest на PS 5.1 игнорирует env
    # HTTP(S)_PROXY — без него запрос шёл НАПРЯМУЮ (ложный успех по гео-403,
    # а при зарезанном DPI прямом коннекте — ложный провал установки).
    $resp = Invoke-WebRequest -Uri "https://api.anthropic.com/v1/messages" `
        -Method POST `
        -UseBasicParsing `
        -Proxy $BridgeUrl `
        -TimeoutSec 15 `
        -ErrorAction Stop
    Write-Warn "API ответил неожиданно ($($resp.StatusCode)). Возможно прокси MITM-ит трафик."
} catch [System.Net.WebException] {
    $code = [int]$_.Exception.Response.StatusCode
    if ($code -eq 401 -or $code -eq 403 -or $code -eq 400) {
        Write-Ok "прокси работает (Anthropic вернул $code — без auth это норма)"
    } elseif ($code -eq 0) {
        Write-Err "не удалось достучаться до Anthropic API"
        Write-Host "        Проверь: прокси из России? Тогда не подойдёт — нужен зарубежный." -ForegroundColor Yellow
        Write-Host "        Ошибка: $($_.Exception.Message)" -ForegroundColor DarkGray
        exit 1
    } else {
        Write-Warn "API ответил $code — может быть прокси работает, но не гарантировано"
    }
} catch {
    # Fallback для PowerShell 7+ где другой тип exception
    $msg = $_.Exception.Message
    if ($msg -match '401|403|400') {
        Write-Ok "прокси работает (без auth Anthropic возвращает 4xx — норма)"
    } else {
        Write-Err "не удалось достучаться: $msg"
        Write-Host "        Проверь что прокси нероссийский." -ForegroundColor Yellow
        exit 1
    }
}

# ─── Шаг 5: ярлык на рабочем столе + скрипт переключения ─────────

Write-Step 5 5 "Создаю ярлык 'Claude Code' на рабочем столе..."

try {
    $desktop = [Environment]::GetFolderPath('Desktop')
    $shortcutPath = Join-Path $desktop 'Claude Code.lnk'

    # Найти где лежит claude.cmd (npm global путь)
    $npmPrefix = (& npm config get prefix 2>$null).Trim()
    $claudeCmd = Join-Path $npmPrefix 'claude.cmd'
    if (-not (Test-Path $claudeCmd)) {
        # На некоторых системах claude может быть просто claude.exe
        $claudeCmd = Join-Path $npmPrefix 'claude'
    }

    $wsh = New-Object -ComObject WScript.Shell
    $sc = $wsh.CreateShortcut($shortcutPath)
    $sc.TargetPath = 'powershell.exe'
    $sc.Arguments = "-NoExit -Command `"& '$claudeCmd'`""
    $sc.WorkingDirectory = [Environment]::GetFolderPath('UserProfile')
    $sc.IconLocation = 'powershell.exe,0'
    $sc.Description = 'Claude Code CLI'
    $sc.Save()

    Write-Ok "ярлык создан"
} catch {
    Write-Warn "не удалось создать ярлык: $_"
    Write-Host "        Это не критично — Claude Code работает через команду 'claude' в терминале." -ForegroundColor DarkGray
}

# ─── Бонус: скрипт переключения прокси ───────────────────────────

$tools = Join-Path ([Environment]::GetFolderPath('UserProfile')) 'claude-tools'
New-Item -ItemType Directory -Path $tools -Force | Out-Null

$switcherPath = Join-Path $tools 'switch-proxy.ps1'
@'
# Быстрая смена прокси для Claude Code и других CLI.
# Запуск: powershell -File switch-proxy.ps1 "http://login:pass@new-ip:port"
param([Parameter(Mandatory)][string]$NewProxy)
[Environment]::SetEnvironmentVariable("HTTPS_PROXY", $NewProxy, "User")
[Environment]::SetEnvironmentVariable("HTTP_PROXY", $NewProxy, "User")
Write-Host "Прокси обновлён на: $NewProxy" -ForegroundColor Green
Write-Host "Закрой все терминалы и открой заново чтобы применить." -ForegroundColor Yellow
'@ | Set-Content -Path $switcherPath -Encoding UTF8

# ─── Final ───────────────────────────────────────────────────────

Write-Host ""
Write-Host "┌─────────────────────────────────────────────────┐" -ForegroundColor Green
Write-Host "│  Готово!                                        │" -ForegroundColor Green
Write-Host "└─────────────────────────────────────────────────┘" -ForegroundColor Green
Write-Host ""
Write-Host "  Что дальше:" -ForegroundColor White
Write-Host ""
Write-Host "  1. " -NoNewline; Write-Host "Закрой все открытые терминалы и VS Code" -ForegroundColor Yellow
Write-Host "     (env-vars подхватываются при старте процесса)" -ForegroundColor DarkGray
Write-Host ""
Write-Host "  2. " -NoNewline; Write-Host "Открой новый PowerShell или дважды кликни ярлык" -ForegroundColor Yellow
Write-Host "     'Claude Code' на рабочем столе."
Write-Host ""
Write-Host "  3. " -NoNewline; Write-Host "Запусти команду: claude" -ForegroundColor Yellow
Write-Host "     При первом запуске спросит как залогиниться:"
Write-Host "       • Claude.ai account — если есть подписка Pro/Max"
Write-Host "       • Anthropic API key — если есть ключ с console.anthropic.com"
Write-Host ""
Write-Host "  Резервный план:" -ForegroundColor DarkGray
Write-Host "  Если основной прокси упадёт — поменяй на резервный командой:" -ForegroundColor DarkGray
Write-Host "    powershell -File `"$switcherPath`" `"http://новый:прокси@ip:port`"" -ForegroundColor DarkGray
Write-Host ""
