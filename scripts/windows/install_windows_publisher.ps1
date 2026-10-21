# Install splitrail on a Windows GPU box serving Ollama (BZL-238): the publisher, which puts
# its usage on the MQTT broker as Ollama@<name>, and the usage proxy in front of Ollama.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File install_windows_publisher.ps1 `
#       -Name vrig -Broker 192.168.163.163 -User splitrail
#
# Expects splitrail-dashboard.exe, ollama_usage_proxy.py, ollama_proxy_vrig.pyw and
# splitrail_publisher.vbs beside it, and the broker password in
# %USERPROFILE%\.config\splitrail\mqtt_password. The proxy task is created DISABLED: enable and
# start it only once Ollama has moved to 127.0.0.1:11435 (OLLAMA_HOST), or both want :11434.
param(
    [Parameter(Mandatory)] [string] $Name,
    [Parameter(Mandatory)] [string] $Broker,
    [Parameter(Mandatory)] [string] $User,
    [string] $Python = "C:\Python311\pythonw.exe",
    [double] $ElectricityPerKwh = 0.24
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$home_ = $env:USERPROFILE
New-Item -ItemType Directory -Force -Path "$home_\.config\splitrail\ollama" | Out-Null

$config = "$home_\.splitrail.toml"
if (-not (Test-Path $config)) {
    # splitrail writes its own complete default (a partial file fails to parse), then this
    # fills in the broker and the electricity.
    & "$here\splitrail-dashboard.exe" config init | Out-Null
    $text = [IO.File]::ReadAllText($config)
    $text = $text -replace "power_watts = 0\.0", "power_watts = 150.0" `
                  -replace "electricity_per_kwh = 0\.0", "electricity_per_kwh = $ElectricityPerKwh" `
                  -replace "\[mqtt\]\r?\n", "[mqtt]`n# This machine on the broker and in other dashboards.`nname = `"$Name`"`nhost = `"$Broker`"`nusername = `"$User`"`n"
    # Without a byte-order mark: Windows PowerShell's UTF8 writes one, and the TOML parser
    # then rejects the whole file.
    [IO.File]::WriteAllText($config, $text, (New-Object Text.UTF8Encoding $false))
}

# The computer name, not USERDOMAIN: over ssh that reads WORKGROUP, which maps to no account.
$me = "$env:COMPUTERNAME\$env:USERNAME"
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
$logon = New-ScheduledTaskTrigger -AtLogOn -User $me
$principal = New-ScheduledTaskPrincipal -UserId $me -LogonType Interactive

$publisher = New-ScheduledTaskAction -Execute "wscript.exe" -Argument "`"$here\splitrail_publisher.vbs`""
Register-ScheduledTask -TaskName "splitrail-publisher" -Action $publisher -Trigger $logon `
    -Settings $settings -Principal $principal -Force | Out-Null

$proxy = New-ScheduledTaskAction -Execute $Python -Argument "`"$here\ollama_proxy_vrig.pyw`"" -WorkingDirectory $here
Register-ScheduledTask -TaskName "splitrail-ollama-proxy" -Action $proxy -Trigger $logon `
    -Settings $settings -Principal $principal -Force | Out-Null
Disable-ScheduledTask -TaskName "splitrail-ollama-proxy" | Out-Null

# Restart the publisher now (a reinstall replaces a running one).
Get-Process splitrail-dashboard -ErrorAction SilentlyContinue | Stop-Process -Force
Start-ScheduledTask -TaskName "splitrail-publisher"
"installed: publisher started as $Name; proxy task disabled until Ollama moves to :11435"
