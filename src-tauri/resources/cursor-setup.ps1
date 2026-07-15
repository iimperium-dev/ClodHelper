#Requires -Version 5.1
<#
.SYNOPSIS
    Подключение Cursor IDE к локальному прокси-bridge (127.0.0.1:8889).

.DESCRIPTION
    ВАЖНО про Cursor: флаг Chromium --proxy-server НЕ заворачивает AI-трафик
    Cursor (он идёт своим Node/HTTP-стеком). Поэтому реальный фикс региона
    («This model provider is not supported in your region») — прописать
    http.proxy в settings.json Cursor. По умолчанию у Cursor
    http.proxySupport = "override" и http.proxy пустой → он игнорирует и
    системный прокси, и env, и --proxy-server. Заполняем http.proxy явно.

      1. Патчит %APPDATA%\Cursor\User\settings.json (с бэкапом):
           http.proxy / https.proxy → мост, proxyStrictSSL=false,
           proxySupport=override, disableHttp2=true.
      2. Создаёт ярлык «Cursor (proxy)» на рабочем столе (просто лаунчер —
         прокси теперь живёт в настройках, применяется к любому запуску).

.PARAMETER Port
    Локальный порт bridge-прокси (по умолчанию 8889).

.PARAMETER Uninstall
    Убрать proxy-ключи из settings.json (восстановить) и удалить ярлык.
#>

param(
    [int]$Port = 8889,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
$BridgeUrl = "http://127.0.0.1:$Port"

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

function Get-DesktopDir {
    $d = [Environment]::GetFolderPath('Desktop')
    if ([string]::IsNullOrEmpty($d)) { $d = Join-Path $env:USERPROFILE 'Desktop' }
    return $d
}

function Find-CursorExe {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\cursor\Cursor.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Cursor\Cursor.exe'),
        (Join-Path $env:PROGRAMFILES 'Cursor\Cursor.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Cursor\Cursor.exe')
    )
    foreach ($p in $candidates) {
        if ($p -and (Test-Path $p)) { return $p }
    }
    foreach ($root in @('HKLM:', 'HKCU:')) {
        $key = "$root\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\Cursor.exe"
        if (Test-Path $key) {
            $val = (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).'(default)'
            if ($val -and (Test-Path $val)) { return $val }
        }
    }
    return $null
}

function Get-CursorSettingsPath {
    return (Join-Path $env:APPDATA 'Cursor\User\settings.json')
}

# Прочитать settings.json в hashtable. JSONC/битый JSON → $null (вызывающий
# код тогда не трогает файл, чтобы не повредить настройки пользователя).
function Read-CursorSettings {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return @{} }
    $raw = Get-Content $Path -Raw -Encoding UTF8
    if ([string]::IsNullOrWhiteSpace($raw)) { return @{} }
    try {
        $obj = $raw | ConvertFrom-Json -ErrorAction Stop
    } catch {
        return $null
    }
    # PSCustomObject → ordered hashtable (сохраняем все существующие ключи).
    $h = [ordered]@{}
    foreach ($p in $obj.PSObject.Properties) { $h[$p.Name] = $p.Value }
    return $h
}

function Save-CursorSettings {
    param([string]$Path, $Settings)
    $dir = Split-Path $Path
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    $json = $Settings | ConvertTo-Json -Depth 100
    # UTF-8 без BOM — как пишет сам VS Code/Cursor.
    [System.IO.File]::WriteAllText($Path, $json, (New-Object System.Text.UTF8Encoding $false))
}

$DesktopShortcut = Join-Path (Get-DesktopDir) 'Cursor (proxy).lnk'
$SettingsPath = Get-CursorSettingsPath
$ProxyKeys = @('http.proxy', 'https.proxy', 'http.proxyStrictSSL', 'http.proxySupport')

# ─── Uninstall ────────────────────────────────────────────────────

if ($Uninstall) {
    Write-Header "Отключение Cursor от прокси"

    Write-Step 1 2 "Убираю proxy-ключи из settings.json..."
    $s = Read-CursorSettings -Path $SettingsPath
    if ($null -eq $s) {
        Write-Warn "settings.json не парсится — оставил как есть. Уберите http.proxy вручную."
    } else {
        Copy-Item $SettingsPath "$SettingsPath.clodhelper.bak" -Force -ErrorAction SilentlyContinue
        foreach ($k in $ProxyKeys) { if ($s.Contains($k)) { $s.Remove($k) } }
        Save-CursorSettings -Path $SettingsPath -Settings $s
        Write-Ok "ключи убраны (бэкап рядом)"
    }

    Write-Step 2 2 "Удаляю ярлык 'Cursor (proxy)'..."
    if (Test-Path $DesktopShortcut) { Remove-Item $DesktopShortcut -Force; Write-Ok "удалён" } else { Write-Ok "не найден" }

    Write-Host ""
    Write-Host "Готово. Cursor больше не ходит через прокси. Перезапустите Cursor." -ForegroundColor DarkGray
    exit 0
}

# ─── Install ──────────────────────────────────────────────────────

Write-Header "Cursor Proxy Setup v2.0"
Write-Host "  Пропишу http.proxy = $BridgeUrl в настройках Cursor."
Write-Host "  Это чинит регион-блок AI (--proxy-server для Cursor не работает)."
Write-Host ""

# Шаг 1 — найти Cursor (для ярлыка и как sanity-check).
Write-Step 1 3 "Ищу Cursor на этом ПК..."
$cursorExe = Find-CursorExe
if (-not $cursorExe) {
    Write-Warn "Cursor не найден"
    Write-Host "        Скачай и установи с https://cursor.com, потом повтори." -ForegroundColor Yellow
    Write-Host "Cursor не найден" -ForegroundColor Red
    exit 0
}
Write-Ok "найден ($cursorExe)"

# Шаг 2 — патчим settings.json.
Write-Step 2 3 "Прописываю прокси в settings.json..."
$s = Read-CursorSettings -Path $SettingsPath
if ($null -eq $s) {
    Write-Err "settings.json есть, но не парсится (возможно с комментариями)."
    Write-Host "        Добавьте вручную в Cursor → settings.json:" -ForegroundColor Yellow
    Write-Host "          `"http.proxy`": `"$BridgeUrl`"," -ForegroundColor DarkGray
    Write-Host "          `"https.proxy`": `"$BridgeUrl`"," -ForegroundColor DarkGray
    Write-Host "          `"http.proxyStrictSSL`": false," -ForegroundColor DarkGray
    Write-Host "          `"http.proxySupport`": `"override`"" -ForegroundColor DarkGray
} else {
    if (Test-Path $SettingsPath) {
        Copy-Item $SettingsPath "$SettingsPath.clodhelper.bak" -Force -ErrorAction SilentlyContinue
    }
    $s['http.proxy'] = $BridgeUrl
    $s['https.proxy'] = $BridgeUrl
    $s['http.proxyStrictSSL'] = $false
    $s['http.proxySupport'] = 'override'
    # HTTP/2 часто ломается через прокси — отключаем (если ещё не).
    if (-not $s.Contains('cursor.general.disableHttp2')) { $s['cursor.general.disableHttp2'] = $true }
    Save-CursorSettings -Path $SettingsPath -Settings $s
    Write-Ok "прокси прописан (бэкап: settings.json.clodhelper.bak)"
}

# Шаг 3 — ярлык-лаунчер на рабочем столе.
Write-Step 3 3 "Создаю ярлык 'Cursor (proxy)' на рабочем столе..."
$wsh = New-Object -ComObject WScript.Shell
$sc = $wsh.CreateShortcut($DesktopShortcut)
$sc.TargetPath = $cursorExe
$sc.WorkingDirectory = Split-Path $cursorExe
$sc.IconLocation = "$cursorExe,0"
$sc.Description = 'Cursor IDE (прокси через настройки http.proxy)'
$sc.Save()
Write-Ok "ярлык создан"

# ─── Final ───────────────────────────────────────────────────────

Write-Host ""
Write-Host "┌─────────────────────────────────────────────────┐" -ForegroundColor Green
Write-Host "│  Готово!                                        │" -ForegroundColor Green
Write-Host "└─────────────────────────────────────────────────┘" -ForegroundColor Green
Write-Host ""
Write-Host "  ВАЖНО: полностью закрой Cursor (все окна и из трея)," -ForegroundColor White
Write-Host "  потом открой заново — настройки прокси применятся при старте." -ForegroundColor White
Write-Host "  Cursor — single-instance: пока висит старый процесс, прокси не применится." -ForegroundColor DarkGray
Write-Host ""
