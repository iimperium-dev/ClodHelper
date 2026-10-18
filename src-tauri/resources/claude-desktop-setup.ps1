#Requires -Version 5.1
<#
.SYNOPSIS
    Настройка Claude Desktop через локальный прокси-bridge.

.DESCRIPTION
    Делает 5 шагов:
      1. Проверяет Python 3.7+ (или py launcher).
      2. Копирует local-proxy.py в %LOCALAPPDATA%\ClaudeDesktopProxy\.
      3. Создаёт автозагрузочный .vbs скрипт (запускает прокси при логине Windows).
      4. Запускает прокси прямо сейчас (без перезагрузки).
      5. Создаёт ярлык «Claude Desktop (proxy)» на рабочем столе.

    После установки Claude Desktop запускается через ярлык и ходит через
    127.0.0.1:8889 → купленный прокси с auth. Не трогает системный прокси.

.PARAMETER ProxyUrl
    URL купленного прокси: http://login:pass@ip:port. Если не задан — спросит.

.PARAMETER Port
    Локальный порт для bridge-прокси (по умолчанию 8889).

.PARAMETER Uninstall
    Удалить bridge: убрать автозагрузку, killнуть процесс, удалить файлы и ярлык.
#>

param(
    [string]$ProxyUrl,
    [int]$Port = 8889,
    [switch]$Uninstall,
    [switch]$Yes,
    # Не создавать ярлык «Claude Desktop (proxy)». Используется когда мост
    # поднимается только ради Claude Code / Cursor, а сам Claude Desktop не нужен.
    [switch]$SkipDesktopShortcut
)

$ErrorActionPreference = 'Stop'
$SourceDir = $PSScriptRoot
$InstallDir = Join-Path $env:LOCALAPPDATA 'ClaudeDesktopProxy'

# Resolve Startup folder. [Environment]::GetFolderPath('Startup') can return ""
# under -NonInteractive PowerShell — fall back to the canonical path.
$StartupDir = [Environment]::GetFolderPath('Startup')
if ([string]::IsNullOrEmpty($StartupDir)) {
    $StartupDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Startup'
}
if (-not (Test-Path $StartupDir)) {
    New-Item -ItemType Directory -Path $StartupDir -Force | Out-Null
}
$StartupVbs = Join-Path $StartupDir 'ClaudeDesktopProxy.vbs'

# Desktop folder — same robustness.
$DesktopDir = [Environment]::GetFolderPath('Desktop')
if ([string]::IsNullOrEmpty($DesktopDir)) {
    $DesktopDir = Join-Path $env:USERPROFILE 'Desktop'
}
$DesktopShortcut = Join-Path $DesktopDir 'Claude Desktop (proxy).lnk'

# ─── Helpers ──────────────────────────────────────────────────────

function Write-Header {
    param([string]$Text)
    Write-Host ""
    Write-Host "┌─────────────────────────────────────────────────┐" -ForegroundColor Cyan
    $padded = $Text.PadRight(47)
    Write-Host "│ $padded │" -ForegroundColor Cyan
    Write-Host "└─────────────────────────────────────────────────┘" -ForegroundColor Cyan
    Write-Host ""
}

function Write-Step { param([int]$N, [int]$Total, [string]$Text); Write-Host ("  [{0}/{1}] " -f $N, $Total) -NoNewline -ForegroundColor DarkGray; Write-Host $Text }
function Write-Ok { param([string]$T = "OK"); Write-Host "        ✓ $T" -ForegroundColor Green }
function Write-Warn { param([string]$T); Write-Host "        ⚠ $T" -ForegroundColor Yellow }
function Write-Err { param([string]$T); Write-Host "        ✗ $T" -ForegroundColor Red }

function Test-ProxyUrl { param([string]$Url); return $Url -match '^https?://[^:@]+:[^@]+@[\d\.\w\-]+:\d+/?$' }

function Get-PythonCommand {
    # На Windows предпочитаем `py` launcher (он точно НЕ Microsoft Store stub).
    try {
        $v = & py --version 2>&1
        if ($LASTEXITCODE -eq 0 -and $v -match 'Python\s+(\d+)\.(\d+)') {
            $major = [int]$matches[1]; $minor = [int]$matches[2]
            if ($major -gt 3 -or ($major -eq 3 -and $minor -ge 7)) {
                return 'py'
            }
        }
    } catch {}
    # Fallback на python (может быть MS Store stub — но тогда вернёт пустой output).
    try {
        $v = & python --version 2>&1
        if ($LASTEXITCODE -eq 0 -and $v -match 'Python\s+(\d+)\.(\d+)') {
            $major = [int]$matches[1]; $minor = [int]$matches[2]
            if ($major -gt 3 -or ($major -eq 3 -and $minor -ge 7)) {
                return 'python'
            }
        }
    } catch {}
    return $null
}

function Stop-ProxyProcess {
    # Найти процесс который слушает наш порт и убить его.
    $procs = Get-NetTCPConnection -State Listen -LocalAddress '127.0.0.1' -LocalPort $Port -ErrorAction SilentlyContinue
    foreach ($conn in $procs) {
        try {
            Stop-Process -Id $conn.OwningProcess -Force -ErrorAction SilentlyContinue
        } catch {}
    }
}

# ─── Uninstall ────────────────────────────────────────────────────

if ($Uninstall) {
    Write-Header "Удаление Claude Desktop Proxy"

    # Do not leave ordinary Store launches pinned to a deleted bridge.
    $policyBase = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')
    $policyKey = $null
    $ownershipKey = $null
    try {
        $policyKey = $policyBase.OpenSubKey('SOFTWARE\Policies\Claude')
        $ownershipKey = $policyBase.OpenSubKey('SOFTWARE\ClodHelper\ClaudeProxyPolicy')
        if (($policyKey -and $policyKey.GetValue('egressProxyUrl') -eq 'http://127.0.0.1:8890') -or
            ($ownershipKey -and $null -ne $ownershipKey.GetValue('State'))) {
            throw 'Сначала откройте Clod Helper → Приложения → Вернуть прежние настройки. Затем повторите удаление моста.'
        }
    } finally {
        if ($policyKey) { $policyKey.Dispose() }
        if ($ownershipKey) { $ownershipKey.Dispose() }
        $policyBase.Dispose()
    }

    Write-Step 1 4 "Останавливаю работающий прокси..."
    Stop-ProxyProcess
    Write-Ok "процесс остановлен"

    Write-Step 2 4 "Удаляю автозагрузку..."
    if (Test-Path $StartupVbs) { Remove-Item $StartupVbs -Force; Write-Ok "удалена" } else { Write-Ok "не найдена" }

    Write-Step 3 4 "Удаляю файлы установки..."
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force; Write-Ok "удалены" } else { Write-Ok "не найдены" }

    Write-Step 4 4 "Удаляю ярлык с рабочего стола..."
    if (Test-Path $DesktopShortcut) { Remove-Item $DesktopShortcut -Force; Write-Ok "удалён" } else { Write-Ok "не найден" }

    Write-Host ""
    Write-Host "Готово. Claude Desktop proxy удалён." -ForegroundColor Green
    Write-Host "Сам Claude Desktop не трогали — он остался установлен." -ForegroundColor DarkGray
    exit 0
}

# ─── Install ──────────────────────────────────────────────────────

Write-Header "Claude Desktop Proxy Setup v1.0"
Write-Host "  Этот скрипт настроит локальный прокси-bridge для Claude Desktop."
Write-Host "  Claude Desktop будет ходить через 127.0.0.1:$Port → купленный прокси."
Write-Host "  Остальные программы (браузер, Steam, Discord) не затронуты."
Write-Host ""

# Шаг 0 — получить прокси
if (-not $ProxyUrl) {
    Write-Host "  Введи URL купленного HTTP-прокси:" -ForegroundColor Yellow
    Write-Host "    Формат: http://login:pass@ip:port" -ForegroundColor DarkGray
    Write-Host ""
    $ProxyUrl = Read-Host "  Прокси"
}

if (-not (Test-ProxyUrl $ProxyUrl)) {
    Write-Err "Прокси не похож на правильный URL: $ProxyUrl"
    exit 1
}

Write-Host ""
Write-Host "  Прокси: $ProxyUrl" -ForegroundColor DarkGray
Write-Host "  Порт локально: 127.0.0.1:$Port" -ForegroundColor DarkGray
if (-not $Yes) {
    $c = Read-Host "  Продолжить? (y/n)"
    if ($c -notmatch '^[yYдД]') { Write-Host "Отменено." -ForegroundColor Yellow; exit 0 }
}

# ─── Шаг 1: Python ───────────────────────────────────────────────

Write-Host ""
Write-Step 1 5 "Проверяю мост clod-bridge.exe..."

# Мост теперь самостоятельный exe (не python.exe). Причина: killswitch блокирует
# python.exe, а мост на python душил сам себя. exe-мост под своим именем не
# попадает под блок. Python больше НЕ требуется.
$sourceBridge = Join-Path $SourceDir 'clod-bridge.exe'
if (-not (Test-Path $sourceBridge)) {
    Write-Err "Не найден clod-bridge.exe рядом с этим скриптом ($sourceBridge)"
    Write-Host "        Переустановите Clod Helper." -ForegroundColor Yellow
    exit 1
}
Write-Ok "найден"

# ─── Шаг 2: копирование clod-bridge.exe ──────────────────────────

Write-Step 2 5 "Копирую clod-bridge.exe в $InstallDir..."

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
$bridgePath = Join-Path $InstallDir 'clod-bridge.exe'
# Остановить работающий мост перед перезаписью exe (иначе файл занят).
Stop-ProxyProcess
Get-Process -Name 'clod-bridge' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
Copy-Item $sourceBridge $bridgePath -Force
Write-Ok "скопировано"

# ─── Шаг 3: автозагрузка через .vbs ──────────────────────────────

Write-Step 3 5 "Настраиваю автозагрузку при логине Windows..."

# .vbs запускает exe напрямую без видимого окна.
$vbsContent = @"
' Auto-start Clod bridge at Windows login (hidden, no console window).
Set sh = CreateObject("WScript.Shell")
sh.Run Chr(34) & "$bridgePath" & Chr(34) & " --upstream " & Chr(34) & "$ProxyUrl" & Chr(34) & " --listen 127.0.0.1:$Port --quiet", 0, False
"@
# Unicode (UTF-16), не ASCII: кириллический путь профиля в ASCII превращается
# в '?', и автозапуск ломается у пользователей с русским именем Windows.
Set-Content -Path $StartupVbs -Value $vbsContent -Encoding Unicode
Write-Ok "автозагрузка прописана ($StartupVbs)"

# ─── Шаг 4: запуск прямо сейчас ──────────────────────────────────

Write-Step 4 5 "Запускаю мост прямо сейчас..."

# Запустить exe через WScript.Shell hidden (тот же способ что vbs).
$wsh = New-Object -ComObject WScript.Shell
$startCmd = "`"$bridgePath`" --upstream `"$ProxyUrl`" --listen 127.0.0.1:$Port --quiet"
$wsh.Run($startCmd, 0, $false) | Out-Null
Start-Sleep -Seconds 2

# Проверить что слушается порт.
$listening = Get-NetTCPConnection -State Listen -LocalAddress '127.0.0.1' -LocalPort $Port -ErrorAction SilentlyContinue
if ($listening) {
    Write-Ok "прокси слушает 127.0.0.1:$Port"
} else {
    Write-Warn "не удалось подтвердить что прокси запустился"
    Write-Host "        Попробуй перелогиниться в Windows (autostart сработает)." -ForegroundColor Yellow
}

# ─── Шаг 5: ярлык на рабочем столе ───────────────────────────────

Write-Step 5 5 "Создаю ярлык 'Claude Desktop (proxy)' на рабочем столе..."

if ($SkipDesktopShortcut) {
    Write-Ok "пропущено (Claude Desktop не выбран)"
    Write-Host ""
    Write-Host "  Мост запущен. Ярлык Claude Desktop не создавали." -ForegroundColor DarkGray
    exit 0
}

$sourceLauncher = Join-Path $SourceDir 'claude-desktop-launch.ps1'
if (-not (Test-Path -LiteralPath $sourceLauncher -PathType Leaf)) {
    Write-Err "Не найден claude-desktop-launch.ps1 рядом с этим скриптом. Переустановите Clod Helper."
    exit 1
}
$launcherPath = Join-Path $InstallDir 'claude-desktop-launch.ps1'
if ([IO.Path]::GetFullPath($sourceLauncher) -ine [IO.Path]::GetFullPath($launcherPath)) {
    Copy-Item -LiteralPath $sourceLauncher -Destination $launcherPath -Force
}
. $launcherPath -Port $Port
$claudeInstallation = Resolve-ClaudeDesktopInstallation
if (-not $claudeInstallation) {
    Write-Warn "Claude Desktop не найден на этом ПК"
    Write-Host "        Скачай и установи с https://claude.ai/download" -ForegroundColor Yellow
    Write-Host "        Потом перезапусти этот скрипт чтобы создать ярлык." -ForegroundColor Yellow
} else {
    $claudeExe = $claudeInstallation.exe_path
    $wsh = New-Object -ComObject WScript.Shell
    $sc = $wsh.CreateShortcut($DesktopShortcut)
    $sc.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $sc.Arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$launcherPath`" -Port 8890 -ShowError"
    $sc.WorkingDirectory = $InstallDir
    $sc.WindowStyle = 7
    $sc.IconLocation = "$claudeExe,0"
    $sc.Description = 'Claude Desktop через локальный прокси'
    $sc.Save()
    Write-Ok "ярлык создан (Claude: $claudeExe)"
}

# ─── Final ───────────────────────────────────────────────────────

Write-Host ""
Write-Host "┌─────────────────────────────────────────────────┐" -ForegroundColor Green
Write-Host "│  Готово!                                        │" -ForegroundColor Green
Write-Host "└─────────────────────────────────────────────────┘" -ForegroundColor Green
Write-Host ""
Write-Host "  Что дальше:" -ForegroundColor White
Write-Host ""
Write-Host "  1. " -NoNewline; Write-Host "В Clod Helper открой Приложения → Настроить Claude." -ForegroundColor Yellow
Write-Host "     Это включает штатный прокси для Claude и агента на порту 8890."
Write-Host "     Затем полностью закрой Claude через трей и запусти через ярлык"
Write-Host "     'Claude Desktop (proxy)' с рабочего стола."
Write-Host "     Штатная настройка применяется также при запуске обычным ярлыком."
Write-Host ""
Write-Host "  2. " -NoNewline; Write-Host "Прокси автоматически запустится при каждом логине Windows" -ForegroundColor Yellow
Write-Host "     (через файл $StartupVbs)."
Write-Host ""
Write-Host "  Сменить прокси:" -ForegroundColor DarkGray
Write-Host "    .\claude-desktop-setup.ps1 -Uninstall" -ForegroundColor DarkGray
Write-Host "    .\claude-desktop-setup.ps1 -ProxyUrl `"http://новый:пароль@ip:port`"" -ForegroundColor DarkGray
Write-Host ""
Write-Host "  Проверить что прокси работает:" -ForegroundColor DarkGray
Write-Host "    netstat -an | findstr `":$Port `"" -ForegroundColor DarkGray
Write-Host ""
