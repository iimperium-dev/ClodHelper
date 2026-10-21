param(
    [ValidateSet('Status', 'Enable', 'Disable')][string]$Action = 'Status',
    [string]$ResultPath = '',
    [switch]$LibraryOnly
)

$ErrorActionPreference = 'Stop'
$script:ClaudePolicyKey = 'SOFTWARE\Policies\Claude'
$script:ClaudeOwnerKey = 'SOFTWARE\ClodHelper\ClaudeProxyPolicy'
$script:ClaudeProxyUrl = 'http://127.0.0.1:8890'

# Use the 64-bit view even when a 32-bit shell invokes this script.
function Read-ClaudeRegistryValue([string]$Key, [string]$Name) {
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

function Write-ClaudeRegistryValue([string]$Key, [string]$Name, [object]$Value) {
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

function Remove-ClaudeRegistryValue([string]$Key, [string]$Name) {
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

function Test-ClaudeValueEqual($Left, $Right) {
    if ([bool]$Left.exists -ne [bool]$Right.exists) { return $false }
    if (-not $Left.exists) { return $true }
    return ($Left.kind -ceq $Right.kind -and $Left.data -ceq $Right.data)
}

function Get-ClaudeOwnedState {
    $stored = Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State'
    if (-not $stored.exists) { return $null }
    if ($stored.kind -cne 'String') { throw 'Запись владения политикой Claude имеет неподдерживаемый тип реестра.' }
    try { $state = $stored.data | ConvertFrom-Json -ErrorAction Stop }
    catch { throw 'Запись владения политикой Claude повреждена; политика не изменена.' }
    # A backup may contain only values that Enable is permitted to replace.
    # Never restore arbitrary registry values from a damaged ownership record.
    if ($state.owner -cne 'ClodHelper' -or $state.version -ne 1 -or
        $state.proxy_url -cne $script:ClaudeProxyUrl -or $null -eq $state.original -or
        $state.original.exists -isnot [bool] -or
        ($state.original.exists -and ($state.original.kind -cne 'String' -or $state.original.data -cne ''))) {
        throw 'Запись владения политикой Claude повреждена; политика не изменена.'
    }
    return $state
}

function Get-ClaudePolicyStatus {
    $fixed = Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl'
    $pac = Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyPacUrl'
    $state = Get-ClaudeOwnedState
    # PAC has precedence. Treat even an empty/malformed value as a conflict;
    # Helper must not guess how a particular Desktop release interprets it.
    $pacActive = $pac.exists
    $ours = $fixed.exists -and $fixed.kind -ceq 'String' -and $fixed.data -ceq $script:ClaudeProxyUrl
    $conflict = $null
    if ($pacActive) {
        $conflict = 'У Claude уже задана PAC-политика с приоритетом над фиксированным прокси. Она сохранена без изменений.'
    } elseif ($null -ne $state -and -not $ours -and -not (Test-ClaudeValueEqual $fixed $state.original)) {
        $conflict = 'Политика прокси Claude изменена вне Helper. Она сохранена без изменений.'
    } elseif ($null -eq $state -and $fixed.exists -and ($fixed.kind -cne 'String' -or $fixed.data -cne '')) {
        $conflict = 'У Claude уже задан фиксированный прокси, который не принадлежит Helper. Он сохранён без изменений.'
    }
    return [pscustomobject]@{
        configured = [bool]($ours -and -not $pacActive)
        managed = [bool]($null -ne $state)
        conflict = $conflict
        proxy_url = $(if ($ours) { $script:ClaudeProxyUrl } else { $null })
        pac_configured = [bool]$pacActive
        notice = $null
    }
}

function Invoke-ClaudeProxyPolicy([string]$Operation) {
    if ($Operation -eq 'Status') { return Get-ClaudePolicyStatus }
    $desired = [pscustomobject]@{ exists = $true; kind = 'String'; data = $script:ClaudeProxyUrl }
    $current = Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl'
    $state = Get-ClaudeOwnedState
    if ($Operation -eq 'Enable') {
        $status = Get-ClaudePolicyStatus
        if ($null -ne $status.conflict) { throw $status.conflict }
        if ($null -eq $state) {
            $state = [pscustomobject]@{
                owner = 'ClodHelper'; version = 1; proxy_url = $script:ClaudeProxyUrl; original = $current
            }
            $backup = [pscustomobject]@{
                exists = $true; kind = 'String'; data = ($state | ConvertTo-Json -Depth 8 -Compress)
            }
            # Persist the backup before touching policy, so a failed write is recoverable.
            Write-ClaudeRegistryValue $script:ClaudeOwnerKey 'State' $backup
            if (-not (Test-ClaudeValueEqual (Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State') $backup)) {
                throw 'Не удалось подтвердить резервную копию политики Claude. Политика прокси не изменена.'
            }
        }
        # Detect an intervening policy writer before replacing the allowed empty value.
        if (-not (Test-ClaudeValueEqual (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl') $current)) {
            throw 'Политика прокси Claude изменилась во время настройки. Проверьте её перед повторной попыткой.'
        }
        Write-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl' $desired
        $status = Get-ClaudePolicyStatus
        if (-not $status.configured -or -not $status.managed -or $null -ne $status.conflict) {
            throw 'Не удалось подтвердить применение фиксированного прокси Claude. Резервная копия сохранена.'
        }
        $status.notice = 'Готово. Перезапустите Claude Desktop (закройте его через значок в трее и откройте снова).'
        return $status
    }
    if ($Operation -ne 'Disable') { throw 'Неизвестная операция с политикой Claude.' }
    if ($null -eq $state) { return Get-ClaudePolicyStatus }
    # Re-read immediately before restoring, after loading the backup. Windows
    # registry writes have no compare-and-swap, so a concurrent administrator
    # can still race the subsequent operation, but an observed edit is preserved.
    $current = Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl'
    $changedExternally = -not (Test-ClaudeValueEqual $current $desired) -and -not (Test-ClaudeValueEqual $current $state.original)
    if (Test-ClaudeValueEqual $current $desired) {
        if ($state.original.exists) {
            Write-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl' $state.original
        } else {
            Remove-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl'
        }
        if (-not (Test-ClaudeValueEqual (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl') $state.original)) {
            throw 'Не удалось подтвердить восстановление политики Claude. Резервная копия сохранена.'
        }
    }
    # An externally changed value belongs to its writer. Do not overwrite it.
    Remove-ClaudeRegistryValue $script:ClaudeOwnerKey 'State'
    if ($null -ne (Get-ClaudeOwnedState)) { throw 'Не удалось удалить запись владения политикой Claude.' }
    $status = Get-ClaudePolicyStatus
    $status.notice = $(if ($changedExternally) {
        'Политика изменена вне Helper и сохранена без изменений. Запись владения Helper удалена.'
    } else {
        'Предыдущая политика прокси Claude восстановлена. Перезапустите Claude Desktop, чтобы применить изменение.'
    })
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
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = New-Object Security.Principal.WindowsPrincipal($identity)
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Для изменения политики Claude требуются права администратора.'
        }
        $mutex = New-Object System.Threading.Mutex($false, 'Global\ClodHelper.ClaudeProxyPolicy')
        try { $locked = $mutex.WaitOne(15000) } catch [System.Threading.AbandonedMutexException] { $locked = $true }
        if (-not $locked) { throw 'Другая операция с политикой Claude ещё выполняется.' }
    }
    $result = @{ ok = $true; status = (Invoke-ClaudeProxyPolicy $Action); error = $null }
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
