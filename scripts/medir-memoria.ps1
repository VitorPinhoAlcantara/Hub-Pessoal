param(
    [ValidateSet("janela", "bandeja", "fechada")]
    [string]$Modo = "janela",
    [ValidateSet("gpu", "software")]
    [string]$Renderizador = "gpu",
    [int]$Espera = 10,
    [switch]$SemRegistro
)

$raiz = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $raiz "target\release\hub.exe"
if (-not (Test-Path $exe)) {
    Write-Error "Compile antes com: cargo build --release -p hub-app"
    exit 1
}

$pastaTeste = Join-Path ([IO.Path]::GetTempPath()) "hub-memoria-$(Get-Random)"
$env:HUB_HOME = $pastaTeste
if ($Renderizador -eq "software") { $env:ICED_BACKEND = "tiny-skia" }

$argumentos = if ($Modo -eq "bandeja") { @("--bandeja") } else { @() }
$processo = if ($argumentos) { Start-Process $exe -ArgumentList $argumentos -PassThru } else { Start-Process $exe -PassThru }
Start-Sleep -Seconds $Espera
if ($Modo -eq "fechada") {
    $processo.CloseMainWindow() | Out-Null
    Start-Sleep -Seconds 4
}
$processo.Refresh()
$cpuInicio = $processo.TotalProcessorTime.TotalMilliseconds
Start-Sleep -Seconds 5
$processo.Refresh()
$cpu = [int]($processo.TotalProcessorTime.TotalMilliseconds - $cpuInicio)
$rss = [int]($processo.WorkingSet64 / 1MB)
$privada = [int]($processo.PrivateMemorySize64 / 1MB)
Stop-Process -Id $processo.Id -ErrorAction SilentlyContinue
Remove-Item Env:ICED_BACKEND -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500
Remove-Item -Recurse -Force $pastaTeste -ErrorAction SilentlyContinue

$data = Get-Date -Format "yyyy-MM-dd HH:mm"
$linha = "| $data | Windows | $Modo | $Renderizador | $rss MB | $privada MB | $cpu ms em 5 s |"
$linha
if (-not $SemRegistro) {
    Add-Content -Path (Join-Path $raiz "PROGRESSO.md") -Value $linha -Encoding utf8
}
