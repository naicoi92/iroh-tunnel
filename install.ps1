<#
.SYNOPSIS
Installs the three iroh-tunnel executables system-wide (issue #93).

.DESCRIPTION
  irm https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.ps1 -OutFile install.ps1
  .\install.ps1            # from an elevated PowerShell

Installs iroh-tunnel.exe, iroh-tunnel-serve.exe and iroh-tunnel-access.exe into
"$env:ProgramFiles\iroh-tunnel". Windows needs no short aliases (owner decision).

Questions are asked before anything is changed. Replacing a different file and
adding the directory to the machine PATH both need a "y"; without an answer
source the answer is no. Declining a replacement stops the install with nothing
changed. Declining the PATH change installs the executables and prints how to
add the directory.

Only checksums are verified. The checksum file comes from the same host as the
archives, so it detects corruption but not a compromised host. Signature
verification waits for the signed release artifacts (#100).

Replacement is staged, then renamed file by file; if one rename fails (for
example a running executable) the others may already be replaced. Rerun after
stopping the process. Downloaded scripts may need
`powershell -ExecutionPolicy Bypass -File install.ps1`.

The installer never registers, starts or changes a service. A running
executable cannot be replaced on Windows: stop it yourself and rerun.

Environment:
  IROH_TUNNEL_BASE_URL      release host, default
                            https://github.com/naicoi92/iroh-tunnel/releases
  IROH_TUNNEL_TARGET        override platform detection, e.g. windows-arm64
  IROH_TUNNEL_PROMPT_INPUT  read answers from this file instead of the console
  IROH_TUNNEL_PATH_FILE     keep the machine PATH in this file instead of the
                            registry (used by tests)

.PARAMETER Version
Install release X (for example 1.2.3) instead of the latest stable release.

.PARAMETER BinDir
Install into this directory instead of "$env:ProgramFiles\iroh-tunnel".

.PARAMETER Yes
Answer yes to every question.

.PARAMETER DryRun
Print what would be installed and change nothing.
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [string]$BinDir = '',
    [switch]$Yes,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$DefaultBaseUrl = 'https://github.com/naicoi92/iroh-tunnel/releases'
$Executables = @('iroh-tunnel', 'iroh-tunnel-serve', 'iroh-tunnel-access')
$MachineEnvKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'

class InstallError : System.Exception {
    InstallError([string]$message) : base($message) {}
}

function Fail([string]$Message) { throw [InstallError]::new($Message) }

# --- answers ----------------------------------------------------------------

# Answers come from IROH_TUNNEL_PROMPT_INPUT or the console. No readable source
# means "no".
$script:Answers = $null
$script:AnswerIndex = 0

function Confirm-Step([string]$Question) {
    if ($Yes) { return $true }
    [Console]::Error.Write("$Question [y/N] ")
    $answer = $null
    if ($env:IROH_TUNNEL_PROMPT_INPUT) {
        if ($null -eq $script:Answers) {
            $script:Answers = @()
            if (Test-Path -LiteralPath $env:IROH_TUNNEL_PROMPT_INPUT -PathType Leaf) {
                $script:Answers = @(Get-Content -LiteralPath $env:IROH_TUNNEL_PROMPT_INPUT)
            }
        }
        if ($script:AnswerIndex -lt $script:Answers.Count) {
            $answer = $script:Answers[$script:AnswerIndex]
            $script:AnswerIndex++
        }
    }
    elseif ([Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
        $answer = Read-Host
    }
    if ($null -eq $answer) {
        [Console]::Error.WriteLine('no terminal to ask; assuming no (use -Yes to accept)')
        return $false
    }
    [Console]::Error.WriteLine('')
    return $answer.Trim() -match '^(y|yes)$'
}

# --- platform ---------------------------------------------------------------

function Get-Target {
    if ($env:IROH_TUNNEL_TARGET) { return $env:IROH_TUNNEL_TARGET }
    if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
        Fail 'install.ps1 installs the Windows executables. Use install.sh on Linux and macOS.'
    }
    # PROCESSOR_ARCHITEW6432 names the real CPU inside a 32-bit process.
    $arch = $env:PROCESSOR_ARCHITEW6432
    if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
    switch ($arch) {
        'AMD64' { return 'windows-amd64' }
        'ARM64' { return 'windows-arm64' }
        default { Fail "unsupported CPU architecture: $arch" }
    }
}

# --- machine PATH -----------------------------------------------------------

function Get-MachinePath {
    if ($env:IROH_TUNNEL_PATH_FILE) {
        if (Test-Path -LiteralPath $env:IROH_TUNNEL_PATH_FILE) {
            return (Get-Content -LiteralPath $env:IROH_TUNNEL_PATH_FILE -Raw).Trim()
        }
        return ''
    }
    # Read the unexpanded value so %SystemRoot%-style entries survive the rewrite.
    $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($MachineEnvKey)
    try { return [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
    finally { $key.Close() }
}

function Set-MachinePath([string]$Value) {
    if ($env:IROH_TUNNEL_PATH_FILE) {
        Set-Content -LiteralPath $env:IROH_TUNNEL_PATH_FILE -Value $Value -NoNewline
    }
    else {
        $key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($MachineEnvKey, $true)
        try { $key.SetValue('Path', $Value, [Microsoft.Win32.RegistryValueKind]::ExpandString) }
        finally { $key.Close() }
        # Tell running programs (Explorer) that the environment changed.
        try {
            $id = "iroh-tunnel-$([Guid]::NewGuid().ToString('N'))"
            [Environment]::SetEnvironmentVariable($id, '1', 'User')
            [Environment]::SetEnvironmentVariable($id, $null, 'User')
        }
        catch { }
    }
}

function Test-OnPath([string]$Path, [string]$Dir) {
    $want = $Dir.TrimEnd('\', '/')
    foreach ($entry in $Path.Split(';')) {
        if ($entry.Trim().TrimEnd('\', '/') -ieq $want) { return $true }
    }
    return $false
}

# --- main -------------------------------------------------------------------

# Everything that changes the system lives here, called on the last line: a
# download cut short does nothing.
function Install-IrohTunnel {
    if ($Version) {
        $Version = $Version.TrimStart('v')
        if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$') {
            Fail '-Version must look like 1.2.3'
        }
    }
    if (-not $BinDir) {
        $programFiles = $env:ProgramFiles
        if (-not $programFiles) { $programFiles = 'C:\Program Files' }
        $BinDir = "$($programFiles.TrimEnd('\', '/'))\iroh-tunnel"
    }

    $target = Get-Target
    $baseUrl = $DefaultBaseUrl
    if ($env:IROH_TUNNEL_BASE_URL) { $baseUrl = $env:IROH_TUNNEL_BASE_URL }
    $baseUrl = $baseUrl.TrimEnd('/')
    if ($Version) { $downloadUrl = "$baseUrl/download/v$Version" }
    else { $downloadUrl = "$baseUrl/latest/download" }

    if ($DryRun) {
        Write-Output "Would install for $target from $downloadUrl into ${BinDir}:"
        foreach ($exe in $Executables) { Write-Output "  $exe.exe" }
        Write-Output 'Would ask before replacing a different file or changing the machine PATH. No service is touched.'
        return
    }

    # Fail on permissions before downloading anything. A directory created for
    # the probe is removed again unless the install completes.
    $binDirExisted = Test-Path -LiteralPath $BinDir
    $completed = $false
    try {
        New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
        $probe = Join-Path $BinDir ".write-test.$PID"
        Set-Content -LiteralPath $probe -Value 'x'
        Remove-Item -LiteralPath $probe -Force
    }
    catch {
        if (-not $binDirExisted) { Remove-Item -LiteralPath $BinDir -Force -ErrorAction SilentlyContinue }
        Fail "cannot write to $BinDir. Rerun from an elevated PowerShell (Run as administrator), or pass -BinDir DIR."
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) "iroh-tunnel-install-$([Guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    $staged = @()
    try {
        if ([Net.ServicePointManager]::SecurityProtocol -notmatch 'Tls12') {
            [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        }
        function Get-File([string]$Url, [string]$Dest) {
            try { Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Dest }
            catch { Fail "download failed: $Url" }
        }

        $sums = Join-Path $tmp 'checksums.txt'
        Get-File "$downloadUrl/checksums.txt" $sums
        $lines = @(Get-Content -LiteralPath $sums)

        if (-not $Version) {
            $pattern = "^[0-9a-fA-F]{64} [ *]*iroh-tunnel-([0-9][^ ]*)-$([regex]::Escape($target))\.zip$"
            foreach ($line in $lines) {
                if ($line -match $pattern) { $Version = $Matches[1]; break }
            }
            if (-not $Version) { Fail "the latest release has no iroh-tunnel archive for $target" }
        }

        # --- download and verify (nothing installed yet) ----------------------
        $extracted = @{}
        foreach ($exe in $Executables) {
            $archive = "$exe-$Version-$target.zip"
            $want = $null
            foreach ($line in $lines) {
                if ($line -match '^([0-9a-fA-F]{64}) [ *]*(\S+)$' -and $Matches[2] -ceq $archive) {
                    $want = $Matches[1].ToLowerInvariant()
                    break
                }
            }
            if (-not $want) { Fail "release $Version does not publish $archive" }
            $file = Join-Path $tmp $archive
            Get-File "$downloadUrl/$archive" $file
            $got = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($got -ne $want) {
                Fail "checksum mismatch for $archive (expected $want, got $got). Nothing was installed."
            }
            $dir = Join-Path $tmp "stage-$exe"
            try { Expand-Archive -LiteralPath $file -DestinationPath $dir -Force }
            catch { Fail "cannot extract $archive" }
            $binary = Join-Path $dir "$exe.exe"
            if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
                Fail "$archive does not contain $exe.exe"
            }
            $extracted[$exe] = $binary
        }

        # --- conflicts and PATH (still nothing installed) ---------------------
        $conflicts = @()
        foreach ($exe in $Executables) {
            $dest = Join-Path $BinDir "$exe.exe"
            if (-not (Test-Path -LiteralPath $dest)) { continue }
            if (Test-Path -LiteralPath $dest -PathType Container) { Fail "$dest is a directory; remove it and rerun." }
            $have = (Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash
            $new = (Get-FileHash -LiteralPath $extracted[$exe] -Algorithm SHA256).Hash
            if ($have -ne $new) { $conflicts += $dest }
        }
        if ($conflicts.Count -gt 0) {
            Write-Output 'These files already exist and differ from what this install would write:'
            foreach ($path in $conflicts) { Write-Output "  $path" }
            if (-not (Confirm-Step 'Replace them?')) { Fail 'declined; nothing was changed.' }
        }

        $machinePath = Get-MachinePath
        $changePath = -not (Test-OnPath $machinePath $BinDir)
        $doPath = $false
        if ($changePath) {
            Write-Output "$BinDir is not on the machine PATH."
            $doPath = Confirm-Step 'Add it to the machine PATH?'
        }

        # --- install ----------------------------------------------------------
        # Stage every file next to its destination first, then rename them all.
        # A failure while staging leaves the installed files untouched.
        foreach ($exe in $Executables) {
            $tmpbin = Join-Path $BinDir ".$exe.exe.new.$PID"
            $staged += $tmpbin
            Copy-Item -LiteralPath $extracted[$exe] -Destination $tmpbin -Force
        }
        foreach ($exe in $Executables) {
            $dest = Join-Path $BinDir "$exe.exe"
            try { Move-Item -LiteralPath (Join-Path $BinDir ".$exe.exe.new.$PID") -Destination $dest -Force }
            catch { Fail "cannot replace $dest. Stop any running iroh-tunnel process and rerun." }
            Write-Output "installed $dest"
        }
        $staged = @()

        if ($doPath) {
            $joined = $machinePath.TrimEnd(';')
            if ($joined) { $joined = "$joined;$BinDir" } else { $joined = $BinDir }
            try {
                Set-MachinePath $joined
                Write-Output 'added to the machine PATH; open a new terminal to use it.'
            }
            catch {
                Write-Output "installed, but the machine PATH was not changed ($($_.Exception.Message)). Run from an elevated PowerShell or add $BinDir to PATH yourself."
            }
        }
        elseif ($changePath) {
            Write-Output "PATH was not changed. Add $BinDir to PATH yourself."
        }

        $completed = $true
        Write-Output "iroh-tunnel $Version installed. No service was registered or changed."
        Write-Output 'Note: only checksums were verified; signature verification is not available yet.'
    }
    finally {
        foreach ($path in $staged) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
        if (-not $completed -and -not $binDirExisted -and (Test-Path -LiteralPath $BinDir) -and
            -not (Get-ChildItem -LiteralPath $BinDir -Force)) {
            Remove-Item -LiteralPath $BinDir -Force -ErrorAction SilentlyContinue
        }
    }
}

try {
    Install-IrohTunnel
}
catch {
    [Console]::Error.WriteLine("error: $($_.Exception.Message)")
    exit 1
}
