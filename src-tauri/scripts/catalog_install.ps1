# Installs a Microsoft Update Catalog driver by UpdateID.
# This runs inside the ALREADY-ELEVATED app instance (no extra UAC), so the user
# only ever saw a UAC prompt for "Fresh Driver" itself.
#
# Placeholders replaced by Rust before execution:
#   __UPDATE_ID__  __CREATE_RESTORE__  __RESULT_FILE__  __PROGRESS_FILE__
$ErrorActionPreference = 'Stop'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }

$UpdateId = '__UPDATE_ID__'
$CreateRestore = '__CREATE_RESTORE__'

function Write-Prog($stage, $percent, $msg) {
  try {
    ([pscustomobject]@{ stage = "$stage"; percent = [int]$percent; message = "$msg" }) |
      ConvertTo-Json -Compress | Set-Content -Path '__PROGRESS_FILE__' -Encoding UTF8
  } catch { }
}
function Write-Result($ok, $reboot, $msg, $published) {
  ([pscustomobject]@{
    success         = [bool]$ok
    reboot_required = [bool]$reboot
    message         = "$msg"
    published_names = @($published | Where-Object { $_ })
  }) | ConvertTo-Json | Set-Content -Path '__RESULT_FILE__' -Encoding UTF8
}

try {
  Write-Prog 'start' 2 'Preparing…'

  # 1. Optional System Restore point.
  if ($CreateRestore -eq '1') {
    Write-Prog 'restore' 6 'Creating restore point…'
    try {
      try {
        New-ItemProperty -Path 'HKLM:\Software\Microsoft\Windows NT\CurrentVersion\SystemRestore' `
          -Name 'SystemRestorePointCreationFrequency' -Value 0 -PropertyType DWord -Force | Out-Null
      } catch { }
      Checkpoint-Computer -Description 'Fresh Driver - before catalog install' -RestorePointType 'DEVICE_DRIVER_INSTALL'
    } catch { }
  }

  # 2. Resolve the real download URL via the catalog DownloadDialog.
  Write-Prog 'resolve' 12 'Resolving download…'
  $body = '[{"size":0,"languages":"","uidInfo":"' + $UpdateId + '","updateID":"' + $UpdateId + '"}]'
  $dlg = Invoke-WebRequest -Uri 'https://catalog.update.microsoft.com/DownloadDialog.aspx' `
    -Method Post -Body @{ updateIDs = $body } -UseBasicParsing -TimeoutSec 60 `
    -UserAgent 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
  $urls = [regex]::Matches($dlg.Content, "downloadInformation\[\d+\]\.files\[\d+\]\.url\s*=\s*'([^']+)'") |
    ForEach-Object { $_.Groups[1].Value }
  if (-not $urls -or @($urls).Count -eq 0) {
    Write-Result $false $false 'Could not resolve a download URL from the catalog.'; exit 0
  }
  $url = @($urls)[0]
  $ext = [System.IO.Path]::GetExtension($url.Split('?')[0])

  # 3. Streamed download with live progress (download spans 20%..70%).
  $work = Join-Path $env:TEMP ('freshdriver_' + $UpdateId)
  if (Test-Path $work) { Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path $work -Force | Out-Null
  $file = Join-Path $work ('driver' + $ext)

  Write-Prog 'download' 20 'Starting download…'
  $req = [System.Net.HttpWebRequest]::Create($url)
  $req.UserAgent = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
  $resp = $req.GetResponse()
  $total = [int64]$resp.ContentLength
  $stream = $resp.GetResponseStream()
  $fs = [System.IO.File]::Create($file)
  try {
    $buf = New-Object byte[] 131072
    $sofar = [int64]0; $last = -1
    while (($n = $stream.Read($buf, 0, $buf.Length)) -gt 0) {
      $fs.Write($buf, 0, $n); $sofar += $n
      if ($total -gt 0) {
        $pct = [int](20 + ($sofar / $total) * 50)
        if ($pct -ne $last) { Write-Prog 'download' $pct ("Downloading… {0:n0}%" -f (($sofar / $total) * 100)); $last = $pct }
      }
    }
  } finally { $fs.Close(); $stream.Close(); $resp.Close() }

  # 4. Install based on package type.
  if ($ext -eq '.cab') {
    Write-Prog 'extract' 74 'Extracting driver package…'
    $ed = Join-Path $work 'extracted'
    New-Item -ItemType Directory -Path $ed -Force | Out-Null
    & expand.exe -F:* "$file" "$ed" | Out-Null
    $infs = Get-ChildItem -Path $ed -Recurse -Filter *.inf -ErrorAction SilentlyContinue
    if (-not $infs -or @($infs).Count -eq 0) {
      Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
      Write-Result $false $false 'No .inf driver found in the downloaded package.'; exit 0
    }
    Write-Prog 'install' 86 'Installing driver…'
    $ok = $false; $reboot = $false; $msgs = @(); $published = @()
    foreach ($inf in $infs) {
      $pnpOut = (& pnputil.exe /add-driver "$($inf.FullName)" /install | Out-String)
      $code = $LASTEXITCODE
      $msgs += ('{0}: exit {1}' -f $inf.Name, $code)
      $m = [regex]::Match($pnpOut, 'oem\d+\.inf')   # the published-name handle for rollback
      if ($m.Success) { $published += $m.Value }
      if ($code -eq 0) { $ok = $true }
      elseif ($code -eq 3010) { $ok = $true; $reboot = $true }
    }
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    Write-Prog 'done' 100 'Finished'
    if ($ok) { Write-Result $true $reboot ('Driver installed from Microsoft Update Catalog. ' + ($msgs -join '; ')) $published }
    else { Write-Result $false $false ('pnputil did not install the driver. ' + ($msgs -join '; ')) @() }
  }
  elseif ($ext -eq '.msu') {
    Write-Prog 'install' 86 'Installing update…'
    $p = Start-Process wusa.exe -ArgumentList @(('"' + $file + '"'), '/quiet', '/norestart') -Wait -PassThru
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    Write-Prog 'done' 100 'Finished'
    if ($p.ExitCode -eq 0) { Write-Result $true $false 'Update installed (.msu).' }
    elseif ($p.ExitCode -eq 3010) { Write-Result $true $true 'Update installed (.msu); reboot required.' }
    else { Write-Result $false $false ('wusa exited with code {0}.' -f $p.ExitCode) }
  }
  else {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    Write-Result $false $false ("Unsupported package type '{0}' - open it in the catalog to install manually." -f $ext)
  }
} catch {
  Write-Prog 'error' 100 'Failed'
  Write-Result $false $false ('Error: ' + $_.Exception.Message)
}
