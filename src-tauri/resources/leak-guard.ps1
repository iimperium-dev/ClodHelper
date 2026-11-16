param(
    [ValidateSet('Status', 'Enable', 'Disable')][string]$Action = 'Status',
    [string]$Services = '',
    [string]$ResultPath = '',
    [switch]$LibraryOnly
)
# «Защита от утечек» Clod Helper (Windows) — то же, что leak-guard.sh (pf) на
# macOS: брандмауэр Windows запрещает ПРЯМОЙ выход к сетям сервисов — Claude
# (Anthropic) и Telegram. Мост ходит к ним только через ваш прокси, поэтому
# правила его не задевают; программа, которая попробует мимо моста, получит
# мгновенный отказ, а не ваш IP.
#
# Правила — исходящие «Блокировать» в своей группе ClodHelper LeakGuard: группу
# ClodHelper целиком снимает прежний killswitch, смешивать их нельзя. Правила
# брандмауэра постоянные — переживают перезагрузку без всякой службы.
#
# WebRTC: Claude Desktop и Codex (Electron) ходят по UDP к STUN-серверам мимо
# любого прокси и так раскрывают настоящий IP; флаги запуска Electron не
# передаёт в процессы страниц (проверено вживую). Поэтому для их exe —
# правило «никакого UDP наружу»: работе через мост UDP не нужен, а DNS идёт
# через системную службу и правилом не задет. Пути exe (у Store-приложений
# они меняются с версией) Helper кладёт рядом в programs.json:
# { "claude": ["C:\...\Claude.exe"], "codex": ["C:\...\ChatGPT.exe"] }.
#
# Использование: leak-guard.ps1 -Action Status|Enable|Disable [-Services claude,telegram,codex] [-ResultPath файл]
# Enable/Disable требуют прав администратора; Status читается без них.
# Результат — одна строка JSON (в stdout или в файл -ResultPath).

$ErrorActionPreference = 'Stop'
$script:LeakGuardGroup = 'ClodHelper LeakGuard'
# Сети сервисов (официальные диапазоны; IPv4 и IPv6) — те же, что в leak-guard.sh.
$script:LeakGuardNets = [ordered]@{
    claude = @('160.79.104.0/21', '2607:6bc0::/48')
    telegram = @('149.154.160.0/20', '91.108.0.0/16', '91.105.192.0/23', '95.161.64.0/20', '185.76.151.0/24', '2001:b28:f23d::/48', '2001:b28:f23f::/48', '2001:67c:4e8::/48', '2a0a:f280::/32')
    # У OpenAI нет своих сетей (общие облачные адреса) — только запрет UDP.
    codex = @()
}
$script:LeakGuardLabels = @{ claude = 'Claude'; telegram = 'Telegram'; codex = 'Codex' }
# Сколько exe одного сервиса закрываем (текущая версия и ещё запущенная старая).
$script:LeakGuardMaxPrograms = 4
$script:LeakGuardProgramsFile = Join-Path $PSScriptRoot 'programs.json'

function Get-LeakGuardRuleName([string]$Service) { return "ClodHelper-LeakGuard-$Service" }
function Get-LeakGuardUdpRuleName([string]$Service, [int]$Index) { return "ClodHelper-LeakGuard-$Service-udp-$Index" }

function Test-LeakGuardRuleActive([string]$Name) {
    $rule = Get-NetFirewallRule -Name $Name -ErrorAction SilentlyContinue
    return ($null -ne $rule -and [string]$rule.Enabled -eq 'True' -and
        [string]$rule.Direction -eq 'Outbound' -and [string]$rule.Action -eq 'Block')
}

# Сервисы, у которых правило на месте и действует (в каноничном порядке).
# Сервис с сетями — по правилу сетей; без сетей (Codex) — по правилу UDP.
function Get-LeakGuardServices {
    $on = @()
    foreach ($service in $script:LeakGuardNets.Keys) {
        if (@($script:LeakGuardNets[$service]).Count -gt 0) {
            $active = Test-LeakGuardRuleActive (Get-LeakGuardRuleName $service)
        } else {
            $active = $false
            for ($i = 0; $i -lt $script:LeakGuardMaxPrograms; $i++) {
                if (Test-LeakGuardRuleActive (Get-LeakGuardUdpRuleName $service $i)) { $active = $true; break }
            }
        }
        if ($active) { $on += $service }
    }
    return ($on -join ',')
}

# Пути exe из programs.json рядом со скриптом: только существующие .exe с
# полным путём, без повторов.
function Get-LeakGuardPrograms([string]$Service) {
    $file = $script:LeakGuardProgramsFile
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { return @() }
    try {
        $map = [IO.File]::ReadAllText($file) | ConvertFrom-Json
    } catch {
        return @()
    }
    $out = @()
    foreach ($path in @($map.$Service)) {
        $path = [string]$path
        if (-not $path -or -not [IO.Path]::IsPathRooted($path) -or
            [IO.Path]::GetExtension($path) -ine '.exe' -or
            -not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
        if ($out -inotcontains $path) { $out += $path }
    }
    return @($out | Select-Object -First $script:LeakGuardMaxPrograms)
}

function Get-LeakGuardStatus {
    $services = Get-LeakGuardServices
    # auto_restore: правила брандмауэра сами переживают перезагрузку.
    return [pscustomobject]@{ enabled = [bool]$services; services = $services; auto_restore = [bool]$services }
}

function Enable-LeakGuard([string]$List) {
    $wanted = @($List -split ',' | Where-Object { $_ })
    if ($wanted.Count -eq 0) { throw 'Не выбраны сервисы для защиты.' }
    foreach ($service in $wanted) {
        if (-not $script:LeakGuardNets.Contains($service)) { throw 'Не выбраны сервисы для защиты.' }
    }
    # Сервису без сетей (Codex) нужен exe — проверяем до того, как снять правила.
    foreach ($service in $wanted) {
        if (@($script:LeakGuardNets[$service]).Count -eq 0 -and @(Get-LeakGuardPrograms $service).Count -eq 0) {
            throw ('{0} не найден на этом компьютере.' -f $script:LeakGuardLabels[$service])
        }
    }
    # С чистого листа: и смена набора сервисов, и починка правил, изменённых вручную.
    Remove-NetFirewallRule -Group $script:LeakGuardGroup -ErrorAction SilentlyContinue
    $expected = @()
    foreach ($service in $script:LeakGuardNets.Keys) {
        if ($wanted -notcontains $service) { continue }
        $label = $script:LeakGuardLabels[$service]
        $programs = @(Get-LeakGuardPrograms $service)
        $nets = @($script:LeakGuardNets[$service])
        if ($nets.Count -gt 0) {
            New-NetFirewallRule -Name (Get-LeakGuardRuleName $service) `
                -DisplayName ('Clod Helper: защита от утечек ({0})' -f $label) `
                -Description 'Запрещает прямой выход к сетям сервиса мимо прокси Clod Helper.' `
                -Group $script:LeakGuardGroup -Direction Outbound -Action Block `
                -RemoteAddress $nets -Profile Any -Enabled True | Out-Null
        }
        $index = 0
        foreach ($program in $programs) {
            New-NetFirewallRule -Name (Get-LeakGuardUdpRuleName $service $index) `
                -DisplayName ('Clod Helper: защита от утечек ({0}, WebRTC)' -f $label) `
                -Description 'Запрещает приложению UDP мимо прокси Clod Helper: WebRTC не раскроет настоящий IP.' `
                -Group $script:LeakGuardGroup -Direction Outbound -Action Block -Protocol UDP `
                -Program $program -Profile Any -Enabled True | Out-Null
            $index++
        }
        $expected += $service
    }
    if ((Get-LeakGuardServices) -cne ($expected -join ',')) { throw 'Не удалось включить правила брандмауэра.' }
}

function Disable-LeakGuard {
    Remove-NetFirewallRule -Group $script:LeakGuardGroup -ErrorAction SilentlyContinue
    if (Get-LeakGuardServices) { throw 'Не удалось снять правила брандмауэра.' }
}

if ($LibraryOnly) { return }
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
$exitCode = 0
try {
    if ($Action -ne 'Status') {
        $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Нужны права администратора.'
        }
    }
    if ($Action -eq 'Enable') { Enable-LeakGuard $Services }
    elseif ($Action -eq 'Disable') { Disable-LeakGuard }
    $result = @{ ok = $true; status = (Get-LeakGuardStatus); error = $null }
} catch {
    $result = @{ ok = $false; status = $null; error = $_.Exception.Message }
    $exitCode = 1
}
$json = $result | ConvertTo-Json -Depth 6 -Compress
if ($ResultPath) {
    [System.IO.File]::WriteAllText($ResultPath, $json, (New-Object System.Text.UTF8Encoding($false)))
} else {
    Write-Output $json
}
exit $exitCode
