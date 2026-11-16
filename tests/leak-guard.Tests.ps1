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
function New-NetFirewallRule($Name, $DisplayName, $Description, $Group, $Direction, $Action, $RemoteAddress, $Profile, $Enabled, $Protocol, $Program) {
    $script:Rules[$Name] = [pscustomobject]@{
        Name = $Name; DisplayName = $DisplayName; Group = $Group; Direction = $Direction
        Action = $Action; RemoteAddress = @($RemoteAddress); Profile = $Profile; Enabled = $Enabled
        Protocol = $Protocol; Program = $Program
    }
    return $script:Rules[$Name]
}
function Remove-NetFirewallRule($Group, $Name, $ErrorAction) {
    foreach ($key in @($script:Rules.Keys)) {
        if ($script:Rules[$key].Group -ceq $Group -or $key -ceq $Name) { $script:Rules.Remove($key) }
    }
}

Assert-Equal ($script:LeakGuardGroup -cne 'ClodHelper') $true 'Leak guard never shares the killswitch rule group'

# Пути exe — во временной папке теста; без programs.json UDP-правил нет.
$script:Tmp = Join-Path ([IO.Path]::GetTempPath()) ('leak-guard-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $script:Tmp | Out-Null
$script:LeakGuardProgramsFile = Join-Path $script:Tmp 'programs.json'

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

# WebRTC: запрет UDP для exe Claude Desktop и Codex.
$claudeExe = Join-Path $script:Tmp 'Claude.exe'
$codexExe = Join-Path $script:Tmp 'ChatGPT.exe'
foreach ($f in @($claudeExe, $codexExe)) { [IO.File]::WriteAllText($f, '') }
$programs = @{ claude = @($claudeExe, $claudeExe, 'relative\x.exe', (Join-Path $script:Tmp 'missing.exe'), (Join-Path $script:Tmp 'notes.txt')); codex = @($codexExe) }
[IO.File]::WriteAllText($script:LeakGuardProgramsFile, ($programs | ConvertTo-Json))
Assert-Throws { Enable-LeakGuard 'claude,calc' } '*Не выбраны*' 'Unknown services are still refused with programs'

Enable-LeakGuard 'claude,codex'
Assert-Equal (Get-LeakGuardServices) 'claude,codex' 'Codex is guarded by its UDP rule alone'
$udp = $script:Rules['ClodHelper-LeakGuard-claude-udp-0']
Assert-Equal $udp.Protocol 'UDP' 'Claude Desktop gets a UDP-only rule'
Assert-Equal $udp.Program $claudeExe 'UDP rule targets the Claude Desktop exe'
Assert-Equal $udp.Action 'Block' 'UDP rule blocks'
Assert-Equal ($null -eq $udp.RemoteAddress -or @($udp.RemoteAddress | Where-Object { $_ }).Count -eq 0) $true 'UDP rule blocks every remote address'
Assert-Equal $script:Rules.ContainsKey('ClodHelper-LeakGuard-claude-udp-1') $false 'Duplicate, relative, missing and non-exe paths are skipped'
Assert-Equal $script:Rules['ClodHelper-LeakGuard-codex-udp-0'].Program $codexExe 'UDP rule targets the Codex exe'
Assert-Equal $script:Rules.ContainsKey('ClodHelper-LeakGuard-codex') $false 'Codex has no network rule: OpenAI has no own networks'
Assert-Equal ($script:Rules['ClodHelper-LeakGuard-claude'].RemoteAddress -join ' ') '160.79.104.0/21 2607:6bc0::/48' 'Claude keeps its network rule next to the UDP rule'

$script:Rules['ClodHelper-LeakGuard-codex-udp-0'].Enabled = 'False'
Assert-Equal (Get-LeakGuardServices) 'claude' 'A disabled UDP rule does not count as Codex protection'

[IO.File]::WriteAllText($script:LeakGuardProgramsFile, '{ "claude": [] }')
Assert-Throws { Enable-LeakGuard 'claude,codex' } '*Codex не найден*' 'Codex without an installed exe is refused'
Assert-Equal (Get-LeakGuardServices) 'claude' 'A refused change keeps the rules that were already there'
Enable-LeakGuard 'claude'
Assert-Equal (Get-LeakGuardServices) 'claude' 'Claude is guarded by networks even without its exe'
[IO.File]::WriteAllText($script:LeakGuardProgramsFile, 'not json')
Enable-LeakGuard 'telegram'
Assert-Equal (Get-LeakGuardServices) 'telegram' 'A broken programs file does not break the network rules'
Disable-LeakGuard
Remove-Item -LiteralPath $script:Tmp -Recurse -Force

Write-Output "PASS: $script:Passed leak guard assertions; no real firewall or UAC operations."
