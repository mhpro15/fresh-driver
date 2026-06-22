# Scans the Microsoft Update Catalog for driver updates newer than what's
# installed, by querying each device's VEN&DEV / VID&PID hardware id concurrently.
#
# For each device it picks the newest catalog driver that matches THIS OS, and
# captures the UpdateID (GUID) needed to later download & install it.
#
# Output: JSON array of available updates (newer than installed) to stdout.
param([int]$MaxDevices = 0, [int]$Throttle = 10)
$ErrorActionPreference = 'Stop'
# Ensure modern TLS for the catalog HTTPS requests (process-wide, applies to runspaces).
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }

# OS family token used to prefer the right catalog package (e.g. "Windows 11" vs "Windows 10").
$build = [int](Get-CimInstance Win32_OperatingSystem).BuildNumber
$osTok = if ($build -ge 22000) { 'Windows 11' } else { 'Windows 10' }

# --- 1. Enumerate installed devices, dedupe by hardware vendor+device id -------
$drivers = Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceName -and $_.HardwareID }
$seen = @{}
$targets = New-Object System.Collections.ArrayList
foreach ($d in $drivers) {
  $hw = @($d.HardwareID)[0]
  $key = $null
  if ($hw -match 'VEN_([0-9A-Fa-f]{4})&DEV_([0-9A-Fa-f]{4})') {
    $key = 'PCI\VEN_{0}&DEV_{1}' -f $matches[1], $matches[2]
  } elseif ($hw -match 'VID_([0-9A-Fa-f]{4})&PID_([0-9A-Fa-f]{4})') {
    $key = 'USB\VID_{0}&PID_{1}' -f $matches[1], $matches[2]
  }
  if (-not $key -or $seen.ContainsKey($key)) { continue }
  $seen[$key] = $true
  [void]$targets.Add([pscustomobject]@{
    query             = $key
    device_name       = $d.DeviceName
    device_class      = $d.DeviceClass
    installed_version = $d.DriverVersion
    os_token          = $osTok
  })
}
if ($MaxDevices -gt 0 -and $targets.Count -gt $MaxDevices) {
  $targets = $targets[0..($MaxDevices - 1)]
}

# --- 2. Worker: query catalog for one hardware id, return newest matching driver
$worker = {
  param($t)
  $res = [pscustomobject]@{
    query = $t.query; device_name = $t.device_name; device_class = $t.device_class
    installed_version = $t.installed_version; available_version = $null
    available_date = $null; update_id = $null; catalog_url = $null; found = $false
  }
  try {
    $url = 'https://www.catalog.update.microsoft.com/Search.aspx?q=' + [uri]::EscapeDataString($t.query)
    $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 40 -UserAgent 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'
    if ($r.Content -notmatch 'No updates matched your search') {
      $rows = [regex]::Matches($r.Content, '(?s)<tr[^>]*id="([a-f0-9\-]{36})_R\d+".*?</tr>')
      $bestVer = $null; $bestDate = $null; $bestUid = $null; $bestScore = -1
      foreach ($m in $rows) {
        $uid = $m.Groups[1].Value
        $cells = [regex]::Matches($m.Value, '(?s)<td[^>]*>(.*?)</td>') | ForEach-Object {
          ($_.Groups[1].Value -replace '(?s)<[^>]+>', '').Trim() -replace '\s+', ' '
        }
        $ne = @($cells | Where-Object { $_ -ne '' })
        $ver  = $ne | Where-Object { $_ -match '^\d+\.\d+(\.\d+){0,2}$' } | Select-Object -First 1
        $date = $ne | Where-Object { $_ -match '^\d{1,2}/\d{1,2}/\d{4}$' } | Select-Object -First 1
        $prod = $ne | Where-Object { $_ -match 'Windows' } | Select-Object -First 1
        if (-not $ver) { continue }
        try { $v = [version]$ver } catch { continue }
        $score = if ($prod -and $prod -match [regex]::Escape($t.os_token)) { 1 } else { 0 }
        # Prefer higher version; for equal versions prefer the OS-matching package.
        $better = $false
        if (-not $bestVer) { $better = $true }
        elseif ($v -gt $bestVer) { $better = $true }
        elseif ($v -eq $bestVer -and $score -gt $bestScore) { $better = $true }
        if ($better) { $bestVer = $v; $bestDate = $date; $bestUid = $uid; $bestScore = $score }
      }
      if ($bestVer) {
        $res.available_version = $bestVer.ToString()
        $res.available_date = $bestDate
        $res.update_id = $bestUid
        $res.catalog_url = $url
        $res.found = $true
      }
    }
  } catch { }
  return $res
}

# --- 3. Run queries concurrently via a runspace pool --------------------------
$pool = [runspacefactory]::CreateRunspacePool(1, $Throttle)
$pool.Open()
$jobs = @()
foreach ($t in $targets) {
  $ps = [powershell]::Create(); $ps.RunspacePool = $pool
  [void]$ps.AddScript($worker).AddArgument($t)
  $jobs += [pscustomobject]@{ PS = $ps; Handle = $ps.BeginInvoke() }
}
$results = New-Object System.Collections.ArrayList
foreach ($j in $jobs) {
  try { foreach ($o in $j.PS.EndInvoke($j.Handle)) { [void]$results.Add($o) } } catch { }
  $j.PS.Dispose()
}
$pool.Close(); $pool.Dispose()

# --- 4. Keep only catalog drivers strictly newer than installed ---------------
$updates = New-Object System.Collections.ArrayList
foreach ($res in $results) {
  if (-not $res.found) { continue }
  try {
    $iv = [version]$res.installed_version
    $av = [version]$res.available_version
    if ($av -gt $iv) { [void]$updates.Add($res) }
  } catch { }
}
ConvertTo-Json -Depth 4 -InputObject @($updates)
