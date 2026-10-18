#Requires -Version 5.1
# All registry operations are mocked. This test never writes HKLM or requests UAC.
$ErrorActionPreference = 'Stop'
$policyPath = Join-Path $PSScriptRoot '..\src-tauri\resources\claude-proxy-policy.ps1'
. $policyPath -LibraryOnly
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

function Reset-Fixture {
    $script:Registry = @{}
    $script:Writes = @()
    $script:FailWrite = ''
    $script:DropWrite = ''
    $script:FailRemove = ''
    $script:ReplaceFixedAfterOwnerRead = $null
}

function Set-FixtureValue([string]$Key, [string]$Name, $Data, [string]$Kind = 'String') {
    $script:Registry["$Key|$Name"] = [pscustomobject]@{ exists = $true; kind = $Kind; data = $Data }
}

function Read-ClaudeRegistryValue([string]$Key, [string]$Name) {
    if ($script:Registry.ContainsKey("$Key|$Name")) {
        $snapshot = ($script:Registry["$Key|$Name"] | ConvertTo-Json -Depth 12 -Compress | ConvertFrom-Json)
        if ($Key -eq $script:ClaudeOwnerKey -and $null -ne $script:ReplaceFixedAfterOwnerRead) {
            Set-FixtureValue $script:ClaudePolicyKey 'egressProxyUrl' $script:ReplaceFixedAfterOwnerRead
            $script:ReplaceFixedAfterOwnerRead = $null
        }
        return $snapshot
    }
    return [pscustomobject]@{ exists = $false; kind = $null; data = $null }
}

function Write-ClaudeRegistryValue([string]$Key, [string]$Name, [object]$Value) {
    $script:Writes += "$Key|$Name"
    if ($script:FailWrite -eq "$Key|$Name") { throw 'Mock write denied' }
    if ($script:DropWrite -eq "$Key|$Name") { return }
    $script:Registry["$Key|$Name"] = ($Value | ConvertTo-Json -Depth 12 -Compress | ConvertFrom-Json)
}

function Remove-ClaudeRegistryValue([string]$Key, [string]$Name) {
    $script:Writes += "remove:$Key|$Name"
    if ($script:FailRemove -eq "$Key|$Name") { throw 'Mock removal denied' }
    $script:Registry.Remove("$Key|$Name")
}

$fixedKey = "$script:ClaudePolicyKey|egressProxyUrl"
$ownerKey = "$script:ClaudeOwnerKey|State"

Reset-Fixture
$state = Invoke-ClaudeProxyPolicy 'Status'
Assert-Equal $state.configured $false 'Missing policy is not configured'
Assert-Equal $state.managed $false 'Missing policy is not owned'
Assert-Equal $script:Writes.Count 0 'Status performs no writes'
Set-FixtureValue $script:ClaudePolicyKey 'unrelatedSetting' 'keep-me'
$state = Invoke-ClaudeProxyPolicy 'Enable'
Assert-Equal $state.configured $true 'Enable writes the fixed proxy'
Assert-Equal $state.managed $true 'Enable records ownership'
Assert-Equal $script:Writes[0] $ownerKey 'Backup is written before the proxy'
Assert-Equal $script:Writes[1] $fixedKey 'Proxy follows the durable backup'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').kind 'String' 'Fixed proxy is REG_SZ'
$savedBackup = (Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State').data
Invoke-ClaudeProxyPolicy 'Enable' | Out-Null
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State').data $savedBackup 'Repeated enable retains original backup'
$state = Invoke-ClaudeProxyPolicy 'Disable'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').exists $false 'Disable restores a missing original value'
Assert-Equal $state.managed $false 'Successful disable removes ownership'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'unrelatedSetting').data 'keep-me' 'Unrelated policy values survive'

Reset-Fixture
Set-FixtureValue $script:ClaudePolicyKey 'egressProxyUrl' ''
Invoke-ClaudeProxyPolicy 'Enable' | Out-Null
Invoke-ClaudeProxyPolicy 'Disable' | Out-Null
$restored = Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl'
Assert-Equal $restored.exists $true 'An originally empty value remains present after disable'
Assert-Equal $restored.data '' 'An originally empty string is restored'
Assert-Equal $restored.kind 'String' 'Original registry type is preserved'

foreach ($foreign in @('http://company-proxy:8080', $script:ClaudeProxyUrl)) {
    Reset-Fixture
    Set-FixtureValue $script:ClaudePolicyKey 'egressProxyUrl' $foreign
    Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*не принадлежит*' 'An unowned nonempty fixed proxy is not overwritten'
    Assert-Equal $script:Writes.Count 0 'Conflicting fixed policy causes zero writes'
    Invoke-ClaudeProxyPolicy 'Disable' | Out-Null
    Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').data $foreign 'Disable preserves unowned policy'
}

foreach ($pac in @('', 'https://company.example/proxy.pac')) {
    Reset-Fixture
    Set-FixtureValue $script:ClaudePolicyKey 'egressProxyPacUrl' $pac
    Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*PAC-*' 'Any present PAC policy prevents enabling a fixed proxy'
    Assert-Equal $script:Writes.Count 0 'PAC conflict causes zero writes'
    Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyPacUrl').data $pac 'PAC policy is preserved'
}

foreach ($kind in @('DWord', 'ExpandString', 'MultiString')) {
    Reset-Fixture
    Set-FixtureValue $script:ClaudePolicyKey 'egressProxyUrl' 0 $kind
    Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*не принадлежит*' 'Unexpected proxy registry types are rejected'
    Assert-Equal $script:Writes.Count 0 'Wrong proxy type causes zero writes'
}

Reset-Fixture
Invoke-ClaudeProxyPolicy 'Enable' | Out-Null
Set-FixtureValue $script:ClaudePolicyKey 'egressProxyUrl' 'http://new-owner:8080'
Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*изменена вне*' 'Enable refuses a policy edited after Helper ownership'
$state = Invoke-ClaudeProxyPolicy 'Disable'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').data 'http://new-owner:8080' 'Disable preserves an external policy edit'
Assert-Equal $state.managed $false 'External edit releases Helper ownership'
Assert-Equal ($state.notice -like '*сохранена без изменений*') $true 'External edit is reported explicitly'

Reset-Fixture
Invoke-ClaudeProxyPolicy 'Enable' | Out-Null
$script:ReplaceFixedAfterOwnerRead = 'http://concurrent-owner:8080'
$state = Invoke-ClaudeProxyPolicy 'Disable'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').data 'http://concurrent-owner:8080' 'Disable re-reads policy after loading backup and preserves an intervening edit'
Assert-Equal ($state.notice -like '*сохранена без изменений*') $true 'An intervening edit is reported explicitly'

Reset-Fixture
Invoke-ClaudeProxyPolicy 'Enable' | Out-Null
Set-FixtureValue $script:ClaudePolicyKey 'egressProxyPacUrl' 'https://new-owner/pac'
$state = Invoke-ClaudeProxyPolicy 'Status'
Assert-Equal $state.configured $false 'PAC introduced after enable invalidates configured status'
Invoke-ClaudeProxyPolicy 'Disable' | Out-Null
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyPacUrl').exists $true 'Disable never removes PAC added by another writer'

Reset-Fixture
$script:FailWrite = $ownerKey
Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*write denied*' 'Backup write failure is surfaced'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').exists $false 'Failed backup leaves proxy unchanged'

Reset-Fixture
$script:DropWrite = $ownerKey
Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*подтвердить резервную копию*' 'Silent backup write failure is detected'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudePolicyKey 'egressProxyUrl').exists $false 'Unverified backup prevents policy change'

Reset-Fixture
$script:FailWrite = $fixedKey
Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*write denied*' 'Proxy write failure is surfaced'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State').exists $true 'Failed apply retains recovery backup'
$script:FailWrite = ''
Invoke-ClaudeProxyPolicy 'Disable' | Out-Null
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State').exists $false 'Disable recovers an interrupted apply'

Reset-Fixture
$script:DropWrite = $fixedKey
Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*подтвердить применение фиксированного прокси*' 'Silent proxy write failure cannot report success'

Reset-Fixture
Invoke-ClaudeProxyPolicy 'Enable' | Out-Null
$script:FailRemove = $fixedKey
Assert-Throws { Invoke-ClaudeProxyPolicy 'Disable' } '*removal denied*' 'Restore failure is surfaced'
Assert-Equal (Read-ClaudeRegistryValue $script:ClaudeOwnerKey 'State').exists $true 'Failed restore keeps ownership backup'

Reset-Fixture
Set-FixtureValue $script:ClaudeOwnerKey 'State' 'invalid-json'
Assert-Throws { Invoke-ClaudeProxyPolicy 'Enable' } '*повреждена*' 'Damaged ownership record prevents writes'
Assert-Equal $script:Writes.Count 0 'Damaged state causes zero writes'
Set-FixtureValue $script:ClaudeOwnerKey 'State' 1 'DWord'
Assert-Throws { Invoke-ClaudeProxyPolicy 'Disable' } '*неподдерживаемый тип*' 'Unexpected ownership registry type prevents restore'
Set-FixtureValue $script:ClaudeOwnerKey 'State' '{"owner":"ClodHelper","version":1,"proxy_url":"http://127.0.0.1:8890","original":{"exists":true,"kind":"String","data":"http://untrusted"}}'
Assert-Throws { Invoke-ClaudeProxyPolicy 'Disable' } '*повреждена*' 'Invalid original backup cannot restore arbitrary policy'

Write-Output "PASS: $script:Passed Claude policy assertions; no real registry or UAC operations."
