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
# Использование: leak-guard.ps1 -Action Status|Enable|Disable [-Services claude,telegram] [-ResultPath файл]
# Enable/Disable требуют прав администратора; Status читается без них.
# Результат — одна строка JSON (в stdout или в файл -ResultPath).

$ErrorActionPreference = 'Stop'
$script:LeakGuardGroup = 'ClodHelper LeakGuard'
# Сети сервисов (официальные диапазоны; IPv4 и IPv6) — те же, что в leak-guard.sh.
$script:LeakGuardNets = [ordered]@{
    claude = @('160.79.104.0/21', '2607:6bc0::/48')
    telegram = @('149.154.160.0/20', '91.108.0.0/16', '91.105.192.0/23', '95.161.64.0/20', '185.76.151.0/24', '2001:b28:f23d::/48', '2001:b28:f23f::/48', '2001:67c:4e8::/48', '2a0a:f280::/32')
}
$script:LeakGuardLabels = @{ claude = 'Claude'; telegram = 'Telegram' }

function Get-LeakGuardRuleName([string]$Service) { return "ClodHelper-LeakGuard-$Service" }

# Сервисы, у которых правило на месте и действует (в каноничном порядке).
function Get-LeakGuardServices {
    $on = @()
    foreach ($service in $script:LeakGuardNets.Keys) {
        $rule = Get-NetFirewallRule -Name (Get-LeakGuardRuleName $service) -ErrorAction SilentlyContinue
        if ($null -ne $rule -and [string]$rule.Enabled -eq 'True' -and
            [string]$rule.Direction -eq 'Outbound' -and [string]$rule.Action -eq 'Block') {
            $on += $service
        }
    }
    return ($on -join ',')
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
    # С чистого листа: и смена набора сервисов, и починка правил, изменённых вручную.
    Remove-NetFirewallRule -Group $script:LeakGuardGroup -ErrorAction SilentlyContinue
    $expected = @()
    foreach ($service in $script:LeakGuardNets.Keys) {
        if ($wanted -notcontains $service) { continue }
        New-NetFirewallRule -Name (Get-LeakGuardRuleName $service) `
            -DisplayName ('Clod Helper: защита от утечек ({0})' -f $script:LeakGuardLabels[$service]) `
            -Description 'Запрещает прямой выход к сетям сервиса мимо прокси Clod Helper.' `
            -Group $script:LeakGuardGroup -Direction Outbound -Action Block `
            -RemoteAddress $script:LeakGuardNets[$service] -Profile Any -Enabled True | Out-Null
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
