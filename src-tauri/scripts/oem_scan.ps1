# Manufacturer-gated OEM driver source. Detects Dell / HP / Lenovo and surfaces
# vendor-official driver updates. Lenovo works tool-free (catalogv2.xml); Dell and
# HP use their own tools (dcu-cli / HP CMSL) when installed; every supported brand
# always gets an official support link. Non-OEM machines return brand=null.
$ErrorActionPreference = 'Stop'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch { }

$cs = Get-CimInstance Win32_ComputerSystem
$mfr = "$($cs.Manufacturer)"
$model = "$($cs.Model)"
$serial = try { (Get-CimInstance Win32_BIOS).SerialNumber } catch { $null }

$result = [pscustomobject]@{ brand = $null; note = $null; support_url = $null; updates = @() }

function Add-Update($name, $category, $version, $url) {
  $result.updates += [pscustomobject]@{
    name = "$name"; category = "$category"; available_version = $version; current_version = $null; download_url = $url
  }
}

try {
  if ($mfr -match 'Dell') {
    $result.brand = 'Dell'
    if ($serial) { $result.support_url = "https://www.dell.com/support/home/en-us/product-support/servicetag/$serial/drivers" }
    $dcu = @("$env:ProgramFiles\Dell\CommandUpdate\dcu-cli.exe", "${env:ProgramFiles(x86)}\Dell\CommandUpdate\dcu-cli.exe") |
      Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($dcu) {
      $rep = Join-Path $env:TEMP 'fd-dcu'
      if (Test-Path $rep) { Remove-Item $rep -Recurse -Force -ErrorAction SilentlyContinue }
      New-Item -ItemType Directory -Path $rep -Force | Out-Null
      try { & $dcu /scan -silent -report="$rep" | Out-Null } catch { }
      $xml = Get-ChildItem "$rep\*.xml" -ErrorAction SilentlyContinue | Select-Object -First 1
      if ($xml) {
        [xml]$d = Get-Content $xml.FullName
        foreach ($u in $d.SelectNodes('//update')) {
          $nm = $u.name; if (-not $nm) { $nm = $u.Attributes['name'].Value }
          Add-Update $nm 'Dell update' $u.version $null
        }
      }
      if ($result.updates.Count -eq 0) { $result.note = 'Dell Command Update found no pending updates (or its report could not be read).' }
    } else {
      $result.note = 'Install Dell Command Update for in-app Dell updates, or use the support link below.'
    }
  }
  elseif ($mfr -match 'HP|Hewlett') {
    $result.brand = 'HP'
    $result.support_url = 'https://support.hp.com/us-en/drivers'
    if (Get-Module -ListAvailable -Name HPCMSL) {
      try {
        Import-Module HPCMSL -ErrorAction Stop
        $sp = Get-SoftpaqList -Category Driver -ErrorAction Stop | Select-Object -First 40
        foreach ($s in $sp) { Add-Update $s.Name 'HP driver' $s.Version $s.Url }
        if ($result.updates.Count -eq 0) { $result.note = 'HP CMSL returned no driver SoftPaqs for this platform.' }
      } catch { $result.note = "HP CMSL error: $($_.Exception.Message)" }
    } else {
      $result.note = 'Install HP CMSL (Install-Module HPCMSL) for in-app HP updates, or use the support link below.'
    }
  }
  elseif ($mfr -match 'LENOVO') {
    $result.brand = 'Lenovo'
    $mt = $null
    $bb = try { (Get-CimInstance Win32_BaseBoard).Product } catch { $null }
    if ($bb -and $bb.Length -ge 4) { $mt = $bb.Substring(0, 4).ToUpper() }
    if ($mt) { $result.support_url = "https://pcsupport.lenovo.com/products/$mt" }
    try {
      [xml]$cat = Invoke-WebRequest 'https://download.lenovo.com/cdrt/td/catalogv2.xml' -UseBasicParsing -TimeoutSec 40
      $node = $cat.ModelList.Model | Where-Object { $_.Types.Type -contains $mt } | Select-Object -First 1
      if ($node) {
        foreach ($s in @($node.SCCM)) {
          if ($s -and $s.'#text') { Add-Update "$($node.name) driver pack ($($s.os))" 'Driver pack' $s.date $s.'#text' }
        }
        $b = @($node.BIOS) | Select-Object -First 1
        if ($b -and $b.'#text') { Add-Update "$($node.name) BIOS $($b.version)" 'BIOS' $b.version $b.'#text' }
      } else {
        $result.note = "No Lenovo catalog entry for machine type $mt - use the support link below."
      }
    } catch { $result.note = "Lenovo catalog unreachable: $($_.Exception.Message)" }
  }
} catch {
  $result.note = "OEM scan error: $($_.Exception.Message)"
}

ConvertTo-Json -Depth 5 -InputObject $result
