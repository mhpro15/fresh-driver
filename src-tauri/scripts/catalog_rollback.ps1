# Rolls back a driver by removing the driver-store package we installed.
# Runs inside the ALREADY-ELEVATED app instance (no extra UAC).
# Placeholders replaced by Rust: __OEM__  __RESULT_FILE__
$ErrorActionPreference = 'Stop'
$oem = '__OEM__'

function Write-Result($ok, $reboot, $msg) {
  ([pscustomobject]@{
    success         = [bool]$ok
    reboot_required = [bool]$reboot
    message         = "$msg"
    published_names = @()
  }) | ConvertTo-Json | Set-Content -Path '__RESULT_FILE__' -Encoding UTF8
}

try {
  # /uninstall removes the package from devices using it; the device then
  # reverts to the previously-installed matching driver. /force allows removal
  # even when the package is in use.
  & pnputil.exe /delete-driver $oem /uninstall /force | Out-Null
  $code = $LASTEXITCODE
  if ($code -eq 0) {
    Write-Result $true $false ("Rolled back: removed $oem and reverted to the previous driver.")
  } elseif ($code -eq 3010) {
    Write-Result $true $true ("Rolled back $oem - a reboot is required to finish.")
  } else {
    Write-Result $false $false ("Could not remove $oem (pnputil exit $code).")
  }
} catch {
  Write-Result $false $false ('Error: ' + $_.Exception.Message)
}
