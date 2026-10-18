#Requires -Version 5.1
# Self-contained regression tests. No installed app/config changes; no app launch.
$ErrorActionPreference = 'Stop'
$launcherPath = Join-Path $PSScriptRoot '..\src-tauri\resources\claude-desktop-launch.ps1'
$savedLocalAppData = $env:LOCALAPPDATA
$savedPath = $env:PATH
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('clod-desktop-tests-' + [guid]::NewGuid().ToString('N'))
$script:Packages = @{}
$script:Manifests = @{}
$script:Processes = @()
$script:RequestedPackages = @()
$script:Passed = 0

function Assert-Equal($Actual, $Expected, [string]$Message) {
    if ($Actual -cne $Expected) { throw "$Message. Expected: [$Expected], actual: [$Actual]" }
    $script:Passed++
    Write-Output "PASS: $Message"
}

function New-FixtureFile([string]$Path) {
    [IO.Directory]::CreateDirectory((Split-Path $Path -Parent)) | Out-Null
    [IO.File]::WriteAllText($Path, '')
}

function Get-AppxPackage {
    [CmdletBinding()]
    param([string]$Name)
    $script:RequestedPackages += $Name
    if ($script:Packages.ContainsKey($Name)) { return $script:Packages[$Name] }
}

function Get-AppxPackageManifest {
    [CmdletBinding()]
    param([string]$Package)
    if ($script:Manifests.ContainsKey($Package)) { return $script:Manifests[$Package] }
    throw 'Fixture manifest not found'
}

function Get-Process {
    [CmdletBinding()]
    param([string]$Name)
    return $script:Processes
}

try {
    [IO.Directory]::CreateDirectory($fixtureRoot) | Out-Null
    $env:LOCALAPPDATA = Join-Path $fixtureRoot 'LocalAppData'
    [IO.Directory]::CreateDirectory($env:LOCALAPPDATA) | Out-Null
    $beforeOutputEncoding = [Console]::OutputEncoding.CodePage
    $importOutput = . $launcherPath
    Assert-Equal $importOutput $null 'Dot-source produces no output'
    Assert-Equal ([Console]::OutputEncoding.CodePage) $beforeOutputEncoding 'Dot-source preserves console encoding'
    Assert-Equal $script:RequestedPackages.Count 0 'Dot-source does not inspect or launch apps'

    $modernRoot = Join-Path $fixtureRoot 'AnthropicPBC.Claude_2.1_x64__test'
    $modernExe = Join-Path $modernRoot 'bin\Claude.exe'
    New-FixtureFile $modernExe
    $script:Packages['AnthropicPBC.Claude'] = [pscustomobject]@{
        InstallLocation = $modernRoot; PackageFullName = 'modern-full'; PackageFamilyName = 'AnthropicPBC.Claude_test'; Version = [version]'2.1'
    }
    $script:Manifests['modern-full'] = [xml]'<Package><Applications><Application Id="Background" Executable="helper.exe"/><Application Id="DesktopClaude" Executable="bin\Claude.exe"/></Applications></Package>'
    $result = Resolve-ClaudeDesktopInstallation
    Assert-Equal $result.exe_path $modernExe 'Store discovery follows manifest executable'
    Assert-Equal $result.app_id 'AnthropicPBC.Claude_test!DesktopClaude' 'Store discovery follows actual manifest application ID'
    Assert-Equal $result.kind 'msix' 'Store install uses msix kind'
    Assert-Equal ($script:RequestedPackages -join ',') 'AnthropicPBC.Claude' 'Modern package has priority'

    $legacyRoot = Join-Path $fixtureRoot 'Claude_1.0_x64__test'
    $legacyExe = Join-Path $legacyRoot 'app\Claude.exe'
    New-FixtureFile $legacyExe
    $script:Packages.Clear()
    $script:Packages['Claude'] = [pscustomobject]@{
        InstallLocation = $legacyRoot; PackageFullName = 'legacy-full'; PackageFamilyName = 'Claude_test'; Version = [version]'1.0'
    }
    $script:Manifests['legacy-full'] = [xml]'<Package><Applications><Application Id="Claude" Executable="app\Claude.exe"/></Applications></Package>'
    $result = Resolve-ClaudeDesktopInstallation
    Assert-Equal $result.exe_path $legacyExe 'Legacy MSIX package remains supported'
    Assert-Equal $result.app_id 'Claude_test!Claude' 'Legacy MSIX app ID is retained'
    $script:Manifests.Clear()
    $result = Resolve-ClaudeDesktopInstallation
    Assert-Equal $result.exe_path $legacyExe 'Known package exe fallback handles missing manifest'
    Assert-Equal $result.app_id $null 'Missing manifest never invents an app ID'

    $script:Packages.Clear()
    $cliDirectory = Join-Path $fixtureRoot 'cli-only'
    New-FixtureFile (Join-Path $cliDirectory 'claude.exe')
    $env:PATH = $cliDirectory + ';' + $savedPath
    $result = Resolve-ClaudeDesktopInstallation
    Assert-Equal $result $null 'Missing Desktop never resolves a Claude Code CLI on PATH'

    $oldSquirrel = Join-Path $env:LOCALAPPDATA 'AnthropicClaude\app-1.9.0\Claude.exe'
    $newSquirrel = Join-Path $env:LOCALAPPDATA 'AnthropicClaude\app-1.10.0\Claude.exe'
    New-FixtureFile $oldSquirrel
    New-FixtureFile $newSquirrel
    $result = Resolve-ClaudeDesktopInstallation
    Assert-Equal $result.exe_path $newSquirrel 'Squirrel discovery chooses newest numeric version'
    Assert-Equal $result.kind 'exe_flag' 'Legacy desktop install uses exe_flag kind'

    $script:Processes = @([pscustomobject]@{ Path = $newSquirrel })
    Assert-Equal (Test-ClaudeDesktopRunning $result) $true 'Running Desktop is recognized'
    $script:Processes = @([pscustomobject]@{ Path = $oldSquirrel })
    Assert-Equal (Test-ClaudeDesktopRunning $result) $true 'Previous Squirrel version still blocks singleton launch'
    $script:Processes = @([pscustomobject]@{ Path = (Join-Path $cliDirectory 'claude.exe') })
    Assert-Equal (Test-ClaudeDesktopRunning $result) $false 'Unrelated CLI process does not block Desktop'

    $proxyBefore = $env:HTTP_PROXY
    $info = New-ClaudeDesktopStartInfo -Installation $result -Port 9999
    Assert-Equal $info.FileName $newSquirrel 'Launch targets the discovered desktop executable'
    Assert-Equal $info.Arguments '--proxy-server=http://127.0.0.1:9999' 'Electron proxy flag uses the selected bridge port'
    Assert-Equal $env:HTTP_PROXY $proxyBefore 'Launch preparation does not mutate caller proxy environment'
    Assert-Equal $info.UseShellExecute $true 'Launch detaches desktop from caller stdout handles'

    # A hidden GUI fixture verifies actual environment inheritance and detached
    # pipe lifetime. ShellExecute waits differently for console apps, so use a
    # GUI executable like Claude. No installed app is started by these tests.
    $childOutput = Join-Path $fixtureRoot 'child-environment.json'
    $childExe = Join-Path $fixtureRoot 'DesktopFixture.exe'
    $childSource = @'
using System;
using System.IO;
using System.Windows.Forms;
public static class DesktopFixture {
    [STAThread]
    public static void Main(string[] args) {
        string[] names = { "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY" };
        string[] values = Array.ConvertAll(names, name => Environment.GetEnvironmentVariable(name));
        File.WriteAllText(args[0] + ".pid", System.Diagnostics.Process.GetCurrentProcess().Id.ToString());
        File.WriteAllLines(args[0], values);
        Timer timer = new Timer();
        timer.Interval = 6000;
        timer.Tick += (sender, e) => Application.ExitThread();
        timer.Start();
        Application.Run();
        timer.Dispose();
    }
}
'@
    Add-Type -TypeDefinition $childSource -ReferencedAssemblies 'System.Windows.Forms.dll' -OutputAssembly $childExe -OutputType WindowsApplication
    $powershellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $parentScript = '. ''' + ([IO.Path]::GetFullPath($launcherPath)).Replace("'", "''") + '''; $info = New-Object Diagnostics.ProcessStartInfo; $info.FileName = ''' + $childExe.Replace("'", "''") + '''; $info.Arguments = ''"' + $childOutput.Replace("'", "''") + '"''; $info.UseShellExecute = $true; $originalProxy = $env:HTTP_PROXY; $child = Start-ClaudeDesktopProcess -StartInfo $info -Port 9999; $child.Dispose(); if ($env:HTTP_PROXY -cne $originalProxy) { throw ''Caller environment was not restored'' }; Write-Output ''parent returned'''
    $parentInfo = New-Object Diagnostics.ProcessStartInfo
    $parentInfo.FileName = $powershellExe
    $parentInfo.Arguments = '-NoProfile -NonInteractive -EncodedCommand ' + [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($parentScript))
    $parentInfo.UseShellExecute = $false
    $parentInfo.CreateNoWindow = $true
    $parentInfo.RedirectStandardOutput = $true
    $parentInfo.RedirectStandardError = $true
    $parent = [Diagnostics.Process]::Start($parentInfo)
    try {
        $parentOutput = $parent.StandardOutput.ReadToEndAsync()
        Assert-Equal ($parentOutput.Wait(3500)) $true 'Parent output completes while detached child is still running'
        Assert-Equal ($parentOutput.Result.Trim()) 'parent returned' 'Caller environment is restored after process creation'
        $childOutputTimer = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $childOutput) -and $childOutputTimer.ElapsedMilliseconds -lt 5000) {
            Start-Sleep -Milliseconds 100
        }
        $childEnvironment = @(Get-Content -LiteralPath $childOutput)
        $proxyNames = @('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY')
        for ($index = 0; $index -lt $proxyNames.Length; $index++) {
            Assert-Equal $childEnvironment[$index] 'http://127.0.0.1:9999' "Child inherits $($proxyNames[$index])"
        }
        Assert-Equal $childEnvironment[3] 'localhost,127.0.0.1,::1' 'Child bypass list contains only loopback'
    } finally { $parent.Dispose() }

    $script:Processes = @([pscustomobject]@{ Path = $newSquirrel })
    $rejected = $false
    try { Invoke-ClaudeDesktopProxy -Port 9999 } catch { $rejected = $_.Exception.Message -match 'Quit' }
    Assert-Equal $rejected $true 'Already running Desktop is rejected with close-from-tray guidance'

    Add-Type -TypeDefinition @'
using System;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading.Tasks;
public sealed class ClaudeHealthFixture : IDisposable {
    readonly TcpListener listener = new TcpListener(IPAddress.Loopback, 0);
    public volatile string Body;
    public int Port { get; private set; }
    public ClaudeHealthFixture() {
        listener.Start(); Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        Task.Run(async () => {
            while (true) {
                TcpClient client;
                try { client = await listener.AcceptTcpClientAsync(); } catch { break; }
                using (client) {
                    try {
                        var stream = client.GetStream(); var request = new byte[4096];
                        await stream.ReadAsync(request, 0, request.Length);
                        var json = (Body ?? "{}").Replace("$PORT", Port.ToString());
                        var bytes = Encoding.UTF8.GetBytes(json);
                        var header = Encoding.ASCII.GetBytes("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: " + bytes.Length + "\r\nConnection: close\r\n\r\n");
                        await stream.WriteAsync(header, 0, header.Length);
                        await stream.WriteAsync(bytes, 0, bytes.Length);
                    } catch { }
                }
            }
        });
    }
    public void Dispose() { listener.Stop(); }
}
'@
    $listener = New-Object ClaudeHealthFixture
    try {
        $testPort = $listener.Port
        $validHealth = '{"service":"clod-bridge","routing_policy":"upstream_only","direct_allowed":false,"strict_ready":true,"mode":"all","listen":"127.0.0.1:$PORT","port":$PORT,"via_direct":0}'
        $listener.Body = $validHealth
        Assert-Equal (Test-ClaudeDesktopBridge -Port $testPort -TimeoutMilliseconds 1500) $true 'Readiness accepts a verified upstream-only bridge'
        foreach ($invalid in @(
            '{}',
            '{"status":"ok","mode":"all"}',
            $validHealth.Replace('"direct_allowed":false', '"direct_allowed":true'),
            $validHealth.Replace('"strict_ready":true', '"strict_ready":false'),
            $validHealth.Replace('"via_direct":0', '"via_direct":1'),
            $validHealth.Replace('"mode":"all"', '"mode":"smart"'),
            $validHealth.Replace('"port":$PORT', '"port":1')
        )) {
            $listener.Body = $invalid
            Assert-Equal (Test-ClaudeDesktopBridge -Port $testPort -TimeoutMilliseconds 300) $false 'Readiness rejects an unverified or downgraded listener'
        }
    } finally { $listener.Dispose() }
    Assert-Equal (Test-ClaudeDesktopBridge -Port $testPort -TimeoutMilliseconds 100) $false 'Bridge readiness rejects a closed loopback port'

    $script:Processes = @()
    function Test-ClaudeDesktopBridge { param([int]$Port) $script:ObservedPort = $Port; return $true }
    function Test-ClaudeDesktopProxyPolicy { param([int]$Port) return $false }
    $rejected = $false
    try { Invoke-ClaudeDesktopProxy -Port 8889 } catch { $rejected = $_.Exception.Message -match 'Clod Helper' }
    Assert-Equal $script:ObservedPort 8890 'Old shortcut migrates to the dedicated Claude port'
    Assert-Equal $rejected $true 'Missing policy prevents launching Claude with a partial configuration'

    Write-Output "All $script:Passed checks passed. No Desktop app was launched."
} finally {
    $env:LOCALAPPDATA = $savedLocalAppData
    $env:PATH = $savedPath
    # Let only our short-lived fixture exit before deleting its executable.
    if ($childOutput -and (Test-Path -LiteralPath ($childOutput + '.pid'))) {
        try {
            $fixtureProcess = [Diagnostics.Process]::GetProcessById([int](Get-Content -LiteralPath ($childOutput + '.pid')))
            if ($fixtureProcess.MainModule.FileName -ieq $childExe) { $fixtureProcess.WaitForExit(8000) | Out-Null }
            $fixtureProcess.Dispose()
        } catch {}
    }
    # Only remove the unique fixture directory under the OS temp directory.
    $resolvedFixture = [IO.Path]::GetFullPath($fixtureRoot)
    $resolvedTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if ($resolvedFixture.StartsWith($resolvedTemp, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path $resolvedFixture -Leaf) -like 'clod-desktop-tests-*') {
        Remove-Item -LiteralPath $resolvedFixture -Recurse -Force -ErrorAction SilentlyContinue
    }
}
