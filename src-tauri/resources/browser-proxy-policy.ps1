param(
    [ValidateSet('Status', 'Enable', 'Disable', 'Apply')][string]$Action = 'Status',
    [string]$Domain = '',
    [string]$Server = '',
    # Apply: несколько браузеров одним окном UAC — домены через запятую.
    [string]$EnableDomains = '',
    [string]$DisableDomains = '',
    [string]$ResultPath = '',
    [switch]$LibraryOnly
)
# «Всегда через мост» для Chromium-браузеров (Windows) — то же, что
# browser-proxy-policy.sh на macOS. Политика HKLM\SOFTWARE\Policies\<браузер>
# главнее параметров запуска и расширений (FoxyProxy, SwitchyOmega): браузер
# идёт через мост, даже открытый из меню «Пуск», а WebRTC не раскрывает IP.
#   ProxySettings    = {"ProxyMode":"fixed_servers","ProxyServer":"<вход моста>","ProxyBypassList":"<local>"}
#   WebRtcIPHandling = disable_non_proxied_udp
#
# Прежние значения сохраняются в записи владения (HKLM\SOFTWARE\ClodHelper\
# BrowserProxyPolicy, имя значения — домен браузера) ДО записи политики и
# возвращаются при снятии. Чужая политика прокси (ProxySettings или прежние
# ProxyMode/ProxyServer/ProxyPacUrl…) и чужой WebRtcIPHandling не
# перезаписываются — это конфликт.
#
# Использование: browser-proxy-policy.ps1 -Action Status|Enable|Disable -Domain <домен> [-Server 127.0.0.1:порт] [-ResultPath файл]
#   домен — com.google.Chrome | com.brave.Browser | com.microsoft.Edge | org.chromium.Chromium
# Enable/Disable требуют прав администратора; Status читается без них.
# Результат — одна строка JSON (в stdout или в файл -ResultPath).

$ErrorActionPreference = 'Stop'
$script:BrowserPolicyKeys = @{
    'com.google.Chrome' = 'SOFTWARE\Policies\Google\Chrome'
    'com.brave.Browser' = 'SOFTWARE\Policies\BraveSoftware\Brave'
    'com.microsoft.Edge' = 'SOFTWARE\Policies\Microsoft\Edge'
    'org.chromium.Chromium' = 'SOFTWARE\Policies\Chromium'
}
$script:BrowserOwnerKey = 'SOFTWARE\ClodHelper\BrowserProxyPolicy'
$script:BrowserWebRtc = 'disable_non_proxied_udp'
# Отдельные политики прокси прежних версий: ProxySettings их перекрывает,
# поэтому любая из них — признак чужой настройки.
$script:BrowserLegacyProxy = @('ProxyMode', 'ProxyServerMode', 'ProxyServer', 'ProxyPacUrl', 'ProxyBypassList')
$script:BrowserForeign = 'У браузера уже есть чужая политика прокси — Clod Helper её не меняет.'

# 64-битное представление реестра — даже из 32-битной оболочки.
function Read-BrowserRegistryValue([string]$Key, [string]$Name) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')
    $handle = $null
    try {
        $handle = $base.OpenSubKey($Key, $false)
        if ($null -eq $handle -or -not ($handle.GetValueNames() -contains $Name)) {
            return [pscustomobject]@{ exists = $false; kind = $null; data = $null }
        }
        return [pscustomobject]@{
            exists = $true
            kind = $handle.GetValueKind($Name).ToString()
            data = $handle.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        }
    } finally {
        if ($null -ne $handle) { $handle.Dispose() }
        $base.Dispose()
    }
}

function Write-BrowserRegistryValue([string]$Key, [string]$Name, [object]$Value) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')
    $handle = $null
    try {
        $handle = $base.CreateSubKey($Key)
        $kind = [System.Enum]::Parse([Microsoft.Win32.RegistryValueKind], [string]$Value.kind)
        $handle.SetValue($Name, $Value.data, $kind)
    } finally {
        if ($null -ne $handle) { $handle.Dispose() }
        $base.Dispose()
    }
}

function Remove-BrowserRegistryValue([string]$Key, [string]$Name) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')
    $handle = $null
    try {
        $handle = $base.OpenSubKey($Key, $true)
        if ($null -ne $handle) { $handle.DeleteValue($Name, $false) }
    } finally {
        if ($null -ne $handle) { $handle.Dispose() }
        $base.Dispose()
    }
}

# Пустой ключ политик браузера после снятия не оставляем.
function Remove-BrowserPolicyKeyIfEmpty([string]$Key) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')
    $handle = $null
    try {
        $handle = $base.OpenSubKey($Key, $false)
        if ($null -eq $handle) { return }
        $empty = $handle.ValueCount -eq 0 -and $handle.SubKeyCount -eq 0
        $handle.Dispose()
        $handle = $null
        if ($empty) { $base.DeleteSubKey($Key, $false) }
    } finally {
        if ($null -ne $handle) { $handle.Dispose() }
        $base.Dispose()
    }
}

# Server — вход моста «127.0.0.1:порт» (весь трафик через мост) или
# «pac:http://127.0.0.1:порт/proxy.pac» («Сайты» только в этом браузере:
# через мост — сайты из списка, остальное напрямую по PAC моста).
function Test-BrowserPac([string]$Server) {
    return ($Server -match '^pac:http://127\.0\.0\.1:\d{1,5}/proxy\.pac$')
}

function Get-BrowserProxySettings([string]$Server) {
    if (Test-BrowserPac $Server) {
        return ('{{"ProxyMode":"pac_script","ProxyPacUrl":"{0}"}}' -f $Server.Substring(4))
    }
    # Строка собирается вручную: ConvertTo-Json превратил бы «<» в <.
    return ('{{"ProxyMode":"fixed_servers","ProxyServer":"{0}","ProxyBypassList":"<local>"}}' -f $Server)
}

function Test-BrowserServer([string]$Server) {
    if ($Server -notmatch '^(?:pac:http://)?127\.0\.0\.1:(\d{1,5})(?:/proxy\.pac)?$') { return $false }
    if (($Server -like 'pac:*') -ne ($Server -like '*/proxy.pac')) { return $false }
    $port = [int]$Matches[1]
    return ($port -ge 1 -and $port -le 65535)
}

# WebRtcIPHandling, который должен стоять при нашей политике. Весь трафик через
# мост — запрет UDP мимо прокси (иначе WebRTC раскрывает IP). PAC — почти всё
# идёт напрямую, и запрет сломал бы звонки на обычных сайтах: оставляем
# исходное значение (как при системной автонастройке «во всех браузерах»).
function Test-BrowserWebRtcOurs($Value, $State) {
    if (Test-BrowserPac $State.server) { return (Test-BrowserValueEqual $Value $State.original.WebRtcIPHandling) }
    return (Test-BrowserString $Value $script:BrowserWebRtc)
}

function Test-BrowserValueEqual($Left, $Right) {
    if ([bool]$Left.exists -ne [bool]$Right.exists) { return $false }
    if (-not $Left.exists) { return $true }
    return ($Left.kind -ceq $Right.kind -and $Left.data -ceq $Right.data)
}

function Test-BrowserString($Value, [string]$Data) {
    return ([bool]$Value.exists -and $Value.kind -ceq 'String' -and $Value.data -ceq $Data)
}

# Исходные значения, которые Enable вправе заменить (и потом вернуть):
# ProxySettings — нет или пустая строка; WebRtcIPHandling — нет или уже наш.
function Test-BrowserOriginal($Value, [string]$Name) {
    if ($null -eq $Value -or $Value.exists -isnot [bool]) { return $false }
    if (-not $Value.exists) { return $true }
    if ($Name -eq 'ProxySettings') { return (Test-BrowserString $Value '') }
    return (Test-BrowserString $Value $script:BrowserWebRtc)
}

function Get-BrowserPolicyKey([string]$Domain) {
    if (-not $script:BrowserPolicyKeys.ContainsKey($Domain)) { throw 'Неизвестный браузер' }
    return $script:BrowserPolicyKeys[$Domain]
}

function Get-BrowserOwnedState([string]$Domain) {
    $stored = Read-BrowserRegistryValue $script:BrowserOwnerKey $Domain
    if (-not $stored.exists) { return $null }
    $damaged = 'Запись владения политикой браузера повреждена; политика не изменена.'
    if ($stored.kind -cne 'String') { throw $damaged }
    try { $state = $stored.data | ConvertFrom-Json -ErrorAction Stop } catch { throw $damaged }
    # Из повреждённой записи никогда не восстанавливаем произвольные значения.
    if ($state.owner -cne 'ClodHelper' -or $state.version -ne 1 -or -not (Test-BrowserServer $state.server) -or
        $null -eq $state.original -or
        -not (Test-BrowserOriginal $state.original.ProxySettings 'ProxySettings') -or
        -not (Test-BrowserOriginal $state.original.WebRtcIPHandling 'WebRtcIPHandling')) {
        throw $damaged
    }
    return $state
}

function Get-BrowserPolicyStatus([string]$Domain) {
    $key = Get-BrowserPolicyKey $Domain
    $proxy = Read-BrowserRegistryValue $key 'ProxySettings'
    $webrtc = Read-BrowserRegistryValue $key 'WebRtcIPHandling'
    $legacy = @($script:BrowserLegacyProxy | Where-Object { (Read-BrowserRegistryValue $key $_).exists })
    $state = Get-BrowserOwnedState $Domain
    $ours = $null -ne $state -and (Test-BrowserString $proxy (Get-BrowserProxySettings $state.server)) -and
        (Test-BrowserWebRtcOurs $webrtc $state)
    $conflict = $null
    if ($legacy.Count -gt 0) {
        $conflict = $script:BrowserForeign
    } elseif ($null -eq $state) {
        if (-not (Test-BrowserOriginal $proxy 'ProxySettings') -or -not (Test-BrowserOriginal $webrtc 'WebRtcIPHandling')) {
            $conflict = $script:BrowserForeign
        }
    } elseif (-not $ours -and -not ((Test-BrowserValueEqual $proxy $state.original.ProxySettings) -and
            (Test-BrowserValueEqual $webrtc $state.original.WebRtcIPHandling))) {
        $conflict = 'Политика прокси браузера изменена вне Clod Helper. Она сохранена без изменений.'
    }
    return [pscustomobject]@{
        configured = [bool]($ours -and $legacy.Count -eq 0)
        managed = [bool]($null -ne $state)
        conflict = $conflict
        # Политики HKLM Windows не стирает — служба восстановления не нужна.
        auto_restore = $false
        server = $(if ($null -ne $state) { [string]$state.server } else { '' })
        notice = $null
    }
}

function Enable-BrowserPolicy([string]$Domain, [string]$Server) {
    if (-not (Test-BrowserServer $Server)) { throw 'Ожидался вход моста 127.0.0.1:порт или PAC моста.' }
    $status = Get-BrowserPolicyStatus $Domain
    if ($null -ne $status.conflict) { throw $status.conflict }
    $key = Get-BrowserPolicyKey $Domain
    $proxy = Read-BrowserRegistryValue $key 'ProxySettings'
    $webrtc = Read-BrowserRegistryValue $key 'WebRtcIPHandling'
    $state = Get-BrowserOwnedState $Domain
    if ($null -eq $state -or $state.server -cne $Server) {
        # Новый вход моста у той же записи — исходные значения прежние.
        $original = $(if ($null -ne $state) { $state.original } else {
            [pscustomobject]@{ ProxySettings = $proxy; WebRtcIPHandling = $webrtc } })
        $state = [pscustomobject]@{ owner = 'ClodHelper'; version = 1; server = $Server; original = $original }
        $backup = [pscustomobject]@{ exists = $true; kind = 'String'; data = ($state | ConvertTo-Json -Depth 8 -Compress) }
        # Сначала запись владения, потом политика — сбой записи восстановим.
        Write-BrowserRegistryValue $script:BrowserOwnerKey $Domain $backup
        if (-not (Test-BrowserValueEqual (Read-BrowserRegistryValue $script:BrowserOwnerKey $Domain) $backup)) {
            throw 'Не удалось сохранить запись владения. Политика браузера не изменена.'
        }
    }
    # Кто-то изменил политику между проверкой и записью — не перетираем.
    if (-not (Test-BrowserValueEqual (Read-BrowserRegistryValue $key 'ProxySettings') $proxy) -or
        -not (Test-BrowserValueEqual (Read-BrowserRegistryValue $key 'WebRtcIPHandling') $webrtc)) {
        throw 'Политика браузера изменилась во время настройки. Проверьте её перед повторной попыткой.'
    }
    Write-BrowserRegistryValue $key 'ProxySettings' ([pscustomobject]@{ exists = $true; kind = 'String'; data = (Get-BrowserProxySettings $Server) })
    if (Test-BrowserPac $Server) {
        # PAC: WebRTC — как было до Clod Helper (переход с «весь трафик» тоже).
        if ($state.original.WebRtcIPHandling.exists) { Write-BrowserRegistryValue $key 'WebRtcIPHandling' $state.original.WebRtcIPHandling }
        else { Remove-BrowserRegistryValue $key 'WebRtcIPHandling' }
    } else {
        Write-BrowserRegistryValue $key 'WebRtcIPHandling' ([pscustomobject]@{ exists = $true; kind = 'String'; data = $script:BrowserWebRtc })
    }
    $status = Get-BrowserPolicyStatus $Domain
    if (-not $status.configured) { throw 'Не удалось подтвердить политику браузера. Запись владения сохранена.' }
    $status.notice = $(if (Test-BrowserPac $Server) {
        'Готово. Перезапустите браузер — сайты из списка пойдут через прокси.'
    } else { 'Готово. Перезапустите браузер — дальше он всегда идёт через мост.' })
    return $status
}

function Disable-BrowserPolicy([string]$Domain) {
    $state = Get-BrowserOwnedState $Domain
    if ($null -eq $state) { return (Get-BrowserPolicyStatus $Domain) }
    $key = Get-BrowserPolicyKey $Domain
    $changedExternally = $false
    $values = @(
        @('ProxySettings', (Get-BrowserProxySettings $state.server), $state.original.ProxySettings),
        @('WebRtcIPHandling', $script:BrowserWebRtc, $state.original.WebRtcIPHandling)
    )
    foreach ($value in $values) {
        $name, $ours, $original = $value
        $current = Read-BrowserRegistryValue $key $name
        if (Test-BrowserString $current $ours) {
            if ($original.exists) { Write-BrowserRegistryValue $key $name $original }
            else { Remove-BrowserRegistryValue $key $name }
            if (-not (Test-BrowserValueEqual (Read-BrowserRegistryValue $key $name) $original)) {
                throw 'Не удалось восстановить политику браузера. Запись владения сохранена.'
            }
        } elseif (-not (Test-BrowserValueEqual $current $original)) {
            # Значение изменено вне Helper и принадлежит тому, кто его записал.
            $changedExternally = $true
        }
    }
    Remove-BrowserPolicyKeyIfEmpty $key
    Remove-BrowserRegistryValue $script:BrowserOwnerKey $Domain
    if ($null -ne (Get-BrowserOwnedState $Domain)) { throw 'Не удалось удалить запись владения политикой браузера.' }
    $status = Get-BrowserPolicyStatus $Domain
    $status.notice = $(if ($changedExternally) {
        'Политика изменена вне Clod Helper и сохранена без изменений. Перезапустите браузер.'
    } else {
        'Политика снята. Перезапустите браузер.'
    })
    return $status
}

# Несколько браузеров за одно окно UAC: сначала снять, потом поставить.
# Возвращает статус последнего; вызывающий перечитывает каждый сам.
function Invoke-BrowserPolicyApply([string]$Enable, [string]$Disable, [string]$Server) {
    $status = $null
    foreach ($d in @($Disable -split ',' | Where-Object { $_ })) { $status = Disable-BrowserPolicy $d }
    foreach ($d in @($Enable -split ',' | Where-Object { $_ })) { $status = Enable-BrowserPolicy $d $Server }
    return $status
}

if ($LibraryOnly) { return }
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8
$exitCode = 0
$mutex = $null
$locked = $false
try {
    if ($Action -ne 'Status') {
        $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Нужны права администратора.'
        }
        $mutex = New-Object System.Threading.Mutex($false, 'Global\ClodHelper.BrowserProxyPolicy')
        try { $locked = $mutex.WaitOne(15000) } catch [System.Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { throw 'Другая операция с политикой браузера ещё выполняется.' }
    }
    $status = switch ($Action) {
        'Enable' { Enable-BrowserPolicy $Domain $Server }
        'Disable' { Disable-BrowserPolicy $Domain }
        'Apply' { Invoke-BrowserPolicyApply $EnableDomains $DisableDomains $Server }
        default { Get-BrowserPolicyStatus $Domain }
    }
    $result = @{ ok = $true; status = $status; error = $null }
} catch {
    $result = @{ ok = $false; status = $null; error = $_.Exception.Message }
    $exitCode = 1
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    if ($null -ne $mutex) { $mutex.Dispose() }
}
$json = $result | ConvertTo-Json -Depth 8 -Compress
if ($ResultPath) {
    [System.IO.File]::WriteAllText($ResultPath, $json, (New-Object System.Text.UTF8Encoding($false)))
} else {
    Write-Output $json
}
exit $exitCode
