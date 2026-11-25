# Runs in the production Windows job, on the final staged installer.
param(
    [Parameter(Mandatory=$true)][string]$Directory,
    [Parameter(Mandatory=$true)][string]$Commit,
    [Parameter(Mandatory=$true)][string]$ExpectedThumbprint
)
$ErrorActionPreference = 'Stop'
if ($Commit -cnotmatch '^[a-f0-9]{40}$' -or $ExpectedThumbprint -cnotmatch '^[A-F0-9]{40}$') {
    throw 'Invalid production source or signer identity'
}
$packages = @(Get-ChildItem -LiteralPath $Directory -File -Filter *.exe)
if ($packages.Count -ne 1) { throw 'Expected exactly one staged Windows installer' }
$package = $packages[0]
if (($package.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $package.Length -le 0) {
    throw 'Unsafe or empty installer'
}
$before = (Get-FileHash -LiteralPath $package.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$size = $package.Length
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe unavailable' }
& $signtool.FullName verify /pa /all /tw $package.FullName
if ($LASTEXITCODE -ne 0) { throw 'Installer signature or timestamp verification failed' }
$signature = Get-AuthenticodeSignature -LiteralPath $package.FullName
if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or
    -not $signature.TimeStamperCertificate -or
    $signature.SignerCertificate.Thumbprint -cne $ExpectedThumbprint) {
    throw 'Installer trust, timestamp or expected signer check failed'
}
if ((Get-FileHash -LiteralPath $package.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $before -or
    (Get-Item -LiteralPath $package.FullName).Length -ne $size) {
    throw 'Installer changed during native verification'
}
$receipt = [ordered]@{
    schema = 'swarmcraft-native-signing-v1'
    commit = $Commit
    platform = 'windows'
    file = $package.Name
    bytes = $size
    sha256 = $before
    checks = @('authenticode_trust', 'all_signatures', 'trusted_timestamp', 'expected_signer')
    identity = $ExpectedThumbprint
}
$destination = Join-Path $Directory 'SIGNING_EVIDENCE_WINDOWS.json'
if (Test-Path -LiteralPath $destination) { throw 'Unexpected pre-existing signing evidence' }
$receipt | ConvertTo-Json -Compress | Set-Content -LiteralPath $destination -Encoding utf8NoBOM
