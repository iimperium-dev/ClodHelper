#Requires -Version 5.1
<#
.SYNOPSIS
    Находит установленный Claude Desktop и запускает его через локальный мост.
.DESCRIPTION
    Путь Microsoft Store меняется при обновлении, поэтому определяется при каждом
    запуске. Dot-source импортирует только функции, без запуска приложения.
#>
param(
    [ValidateRange(1, 65535)]
    [int]$Port = 8890,
    [switch]$DetectOnly,
    [switch]$ShowError
)

function Resolve-ClaudeDesktopInstallation {
    # Modern Store package first; older MSIX builds used the name "Claude".
    foreach ($packageName in @('AnthropicPBC.Claude', 'Claude')) {
        try {
            $packages = @(Get-AppxPackage -Name $packageName -ErrorAction Stop |
                Sort-Object -Property Version -Descending)
            foreach ($package in $packages) {
                if (-not $package.InstallLocation) { continue }
                try {
                    $manifest = Get-AppxPackageManifest -Package $package.PackageFullName -ErrorAction Stop
                    foreach ($application in @($manifest.Package.Applications.Application)) {
                        $executable = [string]$application.Executable
                        if ([IO.Path]::GetFileName($executable) -ine 'Claude.exe') { continue }
                        $exePath = Join-Path $package.InstallLocation $executable
                        if (Test-Path -LiteralPath $exePath -PathType Leaf) {
                            $appId = $null
                            if ($package.PackageFamilyName -and $application.Id) {
                                $appId = '{0}!{1}' -f $package.PackageFamilyName, $application.Id
                            }
                            return [pscustomobject]@{
                                exe_path = $exePath
                                app_id = $appId
                                kind = 'msix'
                            }
                        }
                    }
                } catch {
                    # A damaged/unavailable manifest must not hide a usable exe.
                }
                foreach ($relativePath in @('app\Claude.exe', 'Claude.exe')) {
                    $exePath = Join-Path $package.InstallLocation $relativePath
                    if (Test-Path -LiteralPath $exePath -PathType Leaf) {
                        return [pscustomobject]@{ exe_path = $exePath; app_id = $null; kind = 'msix' }
                    }
                }
            }
        } catch {
            # Get-AppxPackage may be unavailable, e.g. under PowerShell Core.
        }
    }

    # Only known desktop install folders; never resolve the Claude Code CLI from PATH.
    foreach ($relativeRoot in @('AnthropicClaude', 'Claude', 'Programs\claude-desktop', 'Programs\Claude', 'Programs\AnthropicClaude')) {
        $installRoot = Join-Path $env:LOCALAPPDATA $relativeRoot
        $exePath = Join-Path $installRoot 'Claude.exe'
        if (Test-Path -LiteralPath $exePath -PathType Leaf) {
            return [pscustomobject]@{ exe_path = $exePath; app_id = $null; kind = 'exe_flag' }
        }
        # Squirrel installs retain several app-* directories after an update.
        $versions = @(Get-ChildItem -LiteralPath $installRoot -Directory -Filter 'app-*' -ErrorAction SilentlyContinue |
            Sort-Object -Property @{ Expression = {
                try { [version]($_.Name -replace '^app-', '' -replace '-.*$', '') }
                catch { [version]'0.0' }
            } } -Descending)
        foreach ($directory in $versions) {
            $exePath = Join-Path $directory.FullName 'Claude.exe'
            if (Test-Path -LiteralPath $exePath -PathType Leaf) {
                return [pscustomobject]@{ exe_path = $exePath; app_id = $null; kind = 'exe_flag' }
            }
        }
    }
    return $null
}

function Test-ClaudeDesktopRunning {
    param([Parameter(Mandatory = $true)]$Installation)
    foreach ($process in @(Get-Process -Name 'Claude' -ErrorAction SilentlyContinue)) {
        try {
            $processPath = $process.Path
            if (-not $processPath) { continue }
            if ($processPath -ieq $Installation.exe_path) { return $true }
            # A Store update can leave the previous package version running.
            if ($Installation.kind -eq 'msix' -and
                $processPath -match '\\(?:AnthropicPBC\.Claude|Claude)_[^\\]+\\(?:app\\)?Claude\.exe$') {
                return $true
            }
            # The same applies to a previous Squirrel app-* version.
            $currentRoot = Split-Path (Split-Path $Installation.exe_path -Parent) -Parent
            $processRoot = Split-Path (Split-Path $processPath -Parent) -Parent
            if ((Split-Path (Split-Path $Installation.exe_path -Parent) -Leaf) -like 'app-*' -and
                (Split-Path (Split-Path $processPath -Parent) -Leaf) -like 'app-*' -and
                $currentRoot -ieq $processRoot) { return $true }
        } catch {
            # Processes belonging to other users can have inaccessible paths.
        }
    }
    return $false
}

function Test-ClaudeDesktopBridge {
    param([int]$Port, [int]$TimeoutMilliseconds = 4000)
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        $response = $null
        try {
            $request = [Net.HttpWebRequest]::Create("http://127.0.0.1:$Port/healthz")
            $request.Proxy = $null
            $request.AllowAutoRedirect = $false
            $remaining = [Math]::Max(1, $TimeoutMilliseconds - $timer.ElapsedMilliseconds)
            $request.Timeout = [int][Math]::Min(700, $remaining)
            $request.ReadWriteTimeout = $request.Timeout
            $response = $request.GetResponse()
            $reader = New-Object IO.StreamReader($response.GetResponseStream())
            try { $health = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
            if ($health.service -ceq 'clod-bridge' -and
                $health.routing_policy -ceq 'upstream_only' -and
                $health.direct_allowed -is [bool] -and -not $health.direct_allowed -and
                $health.strict_ready -is [bool] -and $health.strict_ready -and
                $health.mode -ceq 'all' -and $health.port -eq $Port -and
                $health.listen -ceq "127.0.0.1:$Port" -and $health.via_direct -eq 0) { return $true }
            return $false
        } catch {
            # The bridge may still be starting; retry within the bounded timeout.
        } finally {
            if ($response) { $response.Dispose() }
        }
        if ($timer.ElapsedMilliseconds -lt $TimeoutMilliseconds) { Start-Sleep -Milliseconds 150 }
    } while ($timer.ElapsedMilliseconds -lt $TimeoutMilliseconds)
    return $false
}

function Test-ClaudeDesktopProxyPolicy {
    param([int]$Port)
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, [Microsoft.Win32.RegistryView]::Registry64)
    $key = $null
    try {
        $key = $base.OpenSubKey('SOFTWARE\Policies\Claude')
        if (-not $key) { return $false }
        if ($key.GetValueNames() -contains 'egressProxyPacUrl') {
            # Even an empty/unknown PAC value must be reconciled by the policy manager.
            return $false
        }
        if ($key.GetValueNames() -notcontains 'egressProxyUrl') { return $false }
        return ($key.GetValueKind('egressProxyUrl') -eq [Microsoft.Win32.RegistryValueKind]::String -and
            $key.GetValue('egressProxyUrl') -ceq "http://127.0.0.1:$Port")
    } finally {
        if ($key) { $key.Dispose() }
        $base.Dispose()
    }
}

function New-ClaudeDesktopStartInfo {
    param([Parameter(Mandatory = $true)]$Installation, [int]$Port)
    $proxy = 'http://127.0.0.1:{0}' -f $Port
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Installation.exe_path
    $startInfo.WorkingDirectory = Split-Path $Installation.exe_path -Parent
    $startInfo.Arguments = '--proxy-server={0}' -f $proxy
    # ShellExecute starts this exact exe detached from the launcher's stdout pipes.
    # UseShellExecute=false can keep Rust Command.output() waiting until Claude exits.
    $startInfo.UseShellExecute = $true
    return $startInfo
}

function Start-ClaudeDesktopProcess {
    param([Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo, [int]$Port)
    $proxy = 'http://127.0.0.1:{0}' -f $Port
    $environment = @{
        HTTP_PROXY = $proxy
        HTTPS_PROXY = $proxy
        ALL_PROXY = $proxy
        NO_PROXY = 'localhost,127.0.0.1,::1'
    }
    $previousEnvironment = @{}
    foreach ($name in $environment.Keys) {
        $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    try {
        # ShellExecute cannot use ProcessStartInfo.EnvironmentVariables. Set only
        # this PowerShell process's environment while creating the child, then restore.
        foreach ($name in $environment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $environment[$name], 'Process')
        }
        return [Diagnostics.Process]::Start($StartInfo)
    } finally {
        foreach ($name in $previousEnvironment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
        }
    }
}

function Start-ClaudeDesktopPackage {
    param([Parameter(Mandatory = $true)][string]$AppId)
    Start-Process -FilePath 'explorer.exe' -ArgumentList ('shell:AppsFolder\' + $AppId)
}

function Invoke-ClaudeDesktopProxy {
    param([int]$Port)
    # Existing shortcuts from older Helper versions explicitly pass 8889.
    # Migrate them to the dedicated listener instead of using shared Smart routing.
    if ($Port -eq 8889) { $Port = 8890 }
    $installation = Resolve-ClaudeDesktopInstallation
    if (-not $installation) {
        throw 'Claude Desktop не найден. Установите приложение из Microsoft Store или с claude.ai/download.'
    }
    if (Test-ClaudeDesktopRunning -Installation $installation) {
        throw 'Claude Desktop уже запущен. Полностью закройте его через значок в трее Windows (Quit / Выход), затем снова запустите через Clod Helper или ярлык Claude Desktop (proxy). Это нужно, чтобы приложение применило прокси.'
    }
    if (-not (Test-ClaudeDesktopBridge -Port $Port)) {
        throw "Мост Claude на 127.0.0.1:$Port не подтвердил работу без DIRECT. Откройте Clod Helper → Приложения → Claude Desktop → «Подключить»."
    }
    if (-not (Test-ClaudeDesktopProxyPolicy -Port $Port)) {
        throw 'Штатный прокси Claude ещё не настроен или изменён другой программой. Откройте Clod Helper → Приложения → Claude Desktop → «Подключить».'
    }
    $startInfo = New-ClaudeDesktopStartInfo -Installation $installation -Port $Port
    try {
        $process = Start-ClaudeDesktopProcess -StartInfo $startInfo -Port $Port
    } catch {
        # Store-версия: на части систем Windows запрещает запускать exe из
        # WindowsApps напрямую («Access is denied»). Прокси уже задан политикой
        # (проверена выше) — запускаем штатно, как из меню «Пуск».
        if (-not $installation.app_id) { throw }
        Start-ClaudeDesktopPackage -AppId $installation.app_id
        return
    }
    if (-not $process) { throw 'Windows не удалось запустить Claude Desktop.' }
    try {
        if ($process.WaitForExit(750) -and $process.ExitCode -ne 0) {
            throw ('Claude Desktop завершился при запуске (код {0}). Полностью закройте Claude через трей и повторите запуск.' -f $process.ExitCode)
        }
    } finally {
        $process.Dispose()
    }
}

# Importing this script must only define functions (setup and regression tests).
if ($MyInvocation.InvocationName -eq '.') { return }

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding

try {
    if ($DetectOnly) {
        $installation = Resolve-ClaudeDesktopInstallation
        if ($installation) { $installation | ConvertTo-Json -Compress }
        else { Write-Output 'null' }
    } else {
        Invoke-ClaudeDesktopProxy -Port $Port
    }
} catch {
    $message = $_.Exception.Message
    [Console]::Error.WriteLine($message)
    if ($ShowError) {
        try {
            Add-Type -AssemblyName PresentationFramework
            [Windows.MessageBox]::Show($message, 'Claude Desktop — прокси', 'OK', 'Warning') | Out-Null
        } catch {}
    }
    exit 1
}
