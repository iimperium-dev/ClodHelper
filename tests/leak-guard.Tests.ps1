#Requires -Version 5.1
# Брандмауэр подменён: тест не трогает правила Windows и не просит UAC.
$ErrorActionPreference = 'Stop'
$guardPath = Join-Path $PSScriptRoot '..\src-tauri\resources\leak-guard.ps1'
. $guardPath -LibraryOnly
$script:Passed = 0

function Assert-Equal($Actual, $Expected, [string]$Message) {
    if ($Actual -cne $Expected) { throw "$Message. Expected: [$Expected], actual: [$Actual]" }
    $script:Passed++
    Write-Output "PASS: $Message"
}

function Assert-Throws([scriptblock]$Code, [string]$Pattern, [string]$Message) {
    try { & $Code | Out-Null } catch {
        if ($_.Exception.Message -notlike $Pattern) { throw "$Message. Unexpected error: $_" }
        $script:Passed++
        Write-Output "PASS: $Message"
        return
    }
    throw "$Message. Expected an error."
}

# Подменённые командлеты брандмауэра: правила — в хеш-таблице по имени.
$script:Rules = @{}
function Get-NetFirewallRule([string]$Name, $ErrorAction) {
    if ($script:Rules.ContainsKey($Name)) { return $script:Rules[$Name] }
}
function New-NetFirewallRule($Name, $DisplayName, $Description, $Group, $Direction, $Action, $RemoteAddress, $Profile, $Enabled) {
    $script:Rules[$Name] = [pscustomobject]@{
        Name = $Name; DisplayName = $DisplayName; Group = $Group; Direction = $Direction
        Action = $Action; RemoteAddress = @($RemoteAddress); Profile = $Profile; Enabled = $Enabled
    }
    return $script:Rules[$Name]
}
function Remove-NetFirewallRule($Group, $Name, $ErrorAction) {
    foreach ($key in @($script:Rules.Keys)) {
        if ($script:Rules[$key].Group -ceq $Group -or $key -ceq $Name) { $script:Rules.Remove($key) }
    }
}

Assert-Equal ($script:LeakGuardGroup -cne 'ClodHelper') $true 'Leak guard never shares the killswitch rule group'

Enable-LeakGuard 'telegram,claude'
$status = Get-LeakGuardStatus
Assert-Equal $status.enabled $true 'Enable turns the guard on'
Assert-Equal $status.services 'claude,telegram' 'Services are reported in canonical order'
Assert-Equal $status.auto_restore $true 'Firewall rules survive a reboot without a service'
$claude = $script:Rules['ClodHelper-LeakGuard-claude']
Assert-Equal $claude.Direction 'Outbound' 'Claude rule blocks outbound traffic'
Assert-Equal $claude.Action 'Block' 'Claude rule action is Block'
Assert-Equal $claude.Profile 'Any' 'Rule applies on every network profile'
Assert-Equal ($claude.RemoteAddress -join ' ') '160.79.104.0/21 2607:6bc0::/48' 'Claude rule covers Anthropic IPv4 and IPv6'
Assert-Equal (@($script:Rules['ClodHelper-LeakGuard-telegram'].RemoteAddress) -contains '149.154.160.0/20') $true 'Telegram rule covers its data centers'
Assert-Equal ($claude.DisplayName -like '*Claude*') $true 'Rule name is readable in Windows Firewall'

Enable-LeakGuard 'telegram'
Assert-Equal (Get-LeakGuardServices) 'telegram' 'A narrower list removes the other rule'
Assert-Equal $script:Rules.ContainsKey('ClodHelper-LeakGuard-claude') $false 'Claude rule is removed'

$script:Rules['ClodHelper-LeakGuard-telegram'].Enabled = 'False'
Assert-Equal (Get-LeakGuardServices) '' 'A disabled rule does not count as protection'
$script:Rules['ClodHelper-LeakGuard-telegram'].Enabled = 'True'
$script:Rules['ClodHelper-LeakGuard-telegram'].Action = 'Allow'
Assert-Equal (Get-LeakGuardServices) '' 'An allow rule does not count as protection'

Assert-Throws { Enable-LeakGuard '' } '*Не выбраны*' 'Empty service list is refused'
Assert-Throws { Enable-LeakGuard 'claude,calc' } '*Не выбраны*' 'Unknown services are refused'

Enable-LeakGuard 'claude'
Disable-LeakGuard
$status = Get-LeakGuardStatus
Assert-Equal $status.enabled $false 'Disable removes protection'
Assert-Equal $script:Rules.Count 0 'Disable removes every rule of the group'

$script:Rules['Other-App-Rule'] = [pscustomobject]@{ Group = 'Other'; Enabled = 'True'; Direction = 'Outbound'; Action = 'Block' }
Enable-LeakGuard 'claude'
Disable-LeakGuard
Assert-Equal $script:Rules.ContainsKey('Other-App-Rule') $true 'Rules of other programs are never touched'

Write-Output "PASS: $script:Passed leak guard assertions; no real firewall or UAC operations."
