#Requires -Version 5.1
# Реестр подменён: тест не пишет в HKLM и не просит UAC.
$ErrorActionPreference = 'Stop'
$policyPath = Join-Path $PSScriptRoot '..\src-tauri\resources\browser-proxy-policy.ps1'
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

$chrome = 'com.google.Chrome'
$chromeKey = 'SOFTWARE\Policies\Google\Chrome'
$ours = '{"ProxyMode":"fixed_servers","ProxyServer":"127.0.0.1:8890","ProxyBypassList":"<local>"}'

function Reset-Fixture {
    $script:Registry = @{}
    $script:Writes = @()
    $script:ForeignWriteOnOwnerSave = $null
}

function Set-FixtureValue([string]$Key, [string]$Name, $Data, [string]$Kind = 'String') {
    $script:Registry["$Key|$Name"] = [pscustomobject]@{ exists = $true; kind = $Kind; data = $Data }
}

function Get-FixtureValue([string]$Key, [string]$Name) { return $script:Registry["$Key|$Name"] }

function Read-BrowserRegistryValue([string]$Key, [string]$Name) {
    if ($script:Registry.ContainsKey("$Key|$Name")) {
        return ($script:Registry["$Key|$Name"] | ConvertTo-Json -Depth 12 -Compress | ConvertFrom-Json)
    }
    return [pscustomobject]@{ exists = $false; kind = $null; data = $null }
}

function Write-BrowserRegistryValue([string]$Key, [string]$Name, [object]$Value) {
    $script:Writes += "$Key|$Name"
    $script:Registry["$Key|$Name"] = ($Value | ConvertTo-Json -Depth 12 -Compress | ConvertFrom-Json)
    # Другой администратор успевает записать политику сразу после записи владения.
    if ($Key -eq $script:BrowserOwnerKey -and $null -ne $script:ForeignWriteOnOwnerSave) {
        Set-FixtureValue $chromeKey 'ProxySettings' $script:ForeignWriteOnOwnerSave
        $script:ForeignWriteOnOwnerSave = $null
    }
}

function Remove-BrowserRegistryValue([string]$Key, [string]$Name) {
    $script:Writes += "remove:$Key|$Name"
    $script:Registry.Remove("$Key|$Name")
}

function Remove-BrowserPolicyKeyIfEmpty([string]$Key) { $script:Writes += "prune:$Key" }

# Нет политик — включаем и снимаем.
Reset-Fixture
$status = Get-BrowserPolicyStatus $chrome
Assert-Equal $status.configured $false 'Nothing is configured initially'
Assert-Equal $status.conflict $null 'No policy means no conflict'
$status = Enable-BrowserPolicy $chrome '127.0.0.1:8890'
Assert-Equal $status.configured $true 'Enable configures Chrome'
Assert-Equal $status.server '127.0.0.1:8890' 'The bridge listener is reported'
Assert-Equal ($status.notice -like '*Перезапустите браузер*') $true 'Enable asks to restart the browser'
Assert-Equal (Get-FixtureValue $chromeKey 'ProxySettings').data $ours 'ProxySettings is the fixed bridge proxy'
Assert-Equal (Get-FixtureValue $chromeKey 'WebRtcIPHandling').data 'disable_non_proxied_udp' 'WebRTC cannot bypass the proxy'
Assert-Equal $script:Writes[0] "$script:BrowserOwnerKey|$chrome" 'Ownership backup is written before the policy'
$status = Disable-BrowserPolicy $chrome
Assert-Equal $status.configured $false 'Disable removes the policy'
Assert-Equal $status.managed $false 'Disable removes the ownership record'
Assert-Equal $script:Registry.Count 0 'Absent values are restored as absent'
Assert-Equal ($script:Writes -contains "prune:$chromeKey") $true 'An empty policy key is pruned'

# Чужая политика прокси не перезаписывается.
Reset-Fixture
Set-FixtureValue $chromeKey 'ProxySettings' '{"ProxyMode":"pac_script","ProxyPacUrl":"https://corp/proxy.pac"}'
$status = Get-BrowserPolicyStatus $chrome
Assert-Equal ($status.conflict -like '*чужая политика*') $true 'Foreign ProxySettings is a conflict'
Assert-Throws { Enable-BrowserPolicy $chrome '127.0.0.1:8890' } '*чужая политика*' 'Enable refuses a foreign proxy policy'
Assert-Equal $script:Writes.Count 0 'A conflict writes nothing'

Reset-Fixture
Set-FixtureValue $chromeKey 'ProxyServer' '10.0.0.1:3128'
Assert-Equal ((Get-BrowserPolicyStatus $chrome).conflict -like '*чужая политика*') $true 'Legacy proxy policies are a conflict'

Reset-Fixture
Set-FixtureValue $chromeKey 'WebRtcIPHandling' 'default'
Assert-Equal ((Get-BrowserPolicyStatus $chrome).conflict -like '*чужая политика*') $true 'Foreign WebRTC policy is a conflict'

# Прежние значения, которые можно заменить, возвращаются при снятии.
Reset-Fixture
Set-FixtureValue $chromeKey 'ProxySettings' ''
Set-FixtureValue $chromeKey 'WebRtcIPHandling' 'disable_non_proxied_udp'
Set-FixtureValue $chromeKey 'HomepageLocation' 'https://example.com'
$null = Enable-BrowserPolicy $chrome '127.0.0.1:8890'
$null = Disable-BrowserPolicy $chrome
Assert-Equal (Get-FixtureValue $chromeKey 'ProxySettings').data '' 'An empty original ProxySettings is restored'
Assert-Equal (Get-FixtureValue $chromeKey 'WebRtcIPHandling').data 'disable_non_proxied_udp' 'An existing WebRTC value is kept'
Assert-Equal (Get-FixtureValue $chromeKey 'HomepageLocation').data 'https://example.com' 'Unrelated policies are untouched'

# Изменение вне Helper после включения — сохраняется.
Reset-Fixture
$null = Enable-BrowserPolicy $chrome '127.0.0.1:8890'
Set-FixtureValue $chromeKey 'ProxySettings' '{"ProxyMode":"direct"}'
Assert-Equal ((Get-BrowserPolicyStatus $chrome).conflict -like '*изменена вне*') $true 'An external edit is reported'
$status = Disable-BrowserPolicy $chrome
Assert-Equal (Get-FixtureValue $chromeKey 'ProxySettings').data '{"ProxyMode":"direct"}' 'An external edit is preserved on disable'
Assert-Equal (Get-FixtureValue $chromeKey 'WebRtcIPHandling') $null 'Our WebRTC value is still removed'
Assert-Equal ($status.notice -like '*изменена вне*') $true 'Disable explains the preserved edit'
Assert-Equal $status.managed $false 'The ownership record is dropped'

# Смена входа моста сохраняет исходные значения.
Reset-Fixture
$null = Enable-BrowserPolicy $chrome '127.0.0.1:8890'
$status = Enable-BrowserPolicy $chrome '127.0.0.1:8892'
Assert-Equal $status.server '127.0.0.1:8892' 'Re-enable switches the bridge listener'
Assert-Equal ((Get-FixtureValue $chromeKey 'ProxySettings').data -like '*127.0.0.1:8892*') $true 'ProxySettings follows the new listener'
$null = Disable-BrowserPolicy $chrome
Assert-Equal $script:Registry.Count 0 'Originals survive a listener switch'

# Гонка: политику записали между проверкой и записью — не перетираем.
Reset-Fixture
$script:ForeignWriteOnOwnerSave = '{"ProxyMode":"system"}'
Assert-Throws { Enable-BrowserPolicy $chrome '127.0.0.1:8890' } '*изменилась во время настройки*' 'An intervening writer stops Enable'
Assert-Equal (Get-FixtureValue $chromeKey 'ProxySettings').data '{"ProxyMode":"system"}' 'The intervening value is kept'

# Проверка входных данных и повреждённой записи владения.
Reset-Fixture
foreach ($bad in @('10.0.0.1:8890', '127.0.0.1:0', '127.0.0.1:70000', '127.0.0.1:8890 x')) {
    Assert-Throws { Enable-BrowserPolicy $chrome $bad } '*127.0.0.1:порт*' "Server $bad is refused"
}
Assert-Throws { Get-BrowserPolicyStatus 'com.example.Unknown' } '*Неизвестный браузер*' 'Unknown browsers are refused'
Set-FixtureValue $script:BrowserOwnerKey $chrome '{"owner":"Someone","version":1}'
Assert-Throws { Get-BrowserPolicyStatus $chrome } '*повреждена*' 'A damaged ownership record is never trusted'
Set-FixtureValue $script:BrowserOwnerKey $chrome '{"owner":"ClodHelper","version":1,"server":"127.0.0.1:8890","original":{"ProxySettings":{"exists":true,"kind":"String","data":"{\"ProxyMode\":\"direct\"}"},"WebRtcIPHandling":{"exists":false,"kind":null,"data":null}}}'
Assert-Throws { Get-BrowserPolicyStatus $chrome } '*повреждена*' 'A backup cannot restore an arbitrary proxy policy'

# Другие браузеры — свои ключи политик.
Reset-Fixture
$null = Enable-BrowserPolicy 'com.microsoft.Edge' '127.0.0.1:8890'
Assert-Equal (Get-FixtureValue 'SOFTWARE\Policies\Microsoft\Edge' 'ProxySettings').data $ours 'Edge policy key'
$null = Enable-BrowserPolicy 'com.brave.Browser' '127.0.0.1:8890'
Assert-Equal (Get-FixtureValue 'SOFTWARE\Policies\BraveSoftware\Brave' 'ProxySettings').data $ours 'Brave policy key'
Assert-Equal (Get-BrowserPolicyStatus $chrome).configured $false 'Browsers are independent'

Write-Output "PASS: $script:Passed browser policy assertions; no real registry or UAC operations."
