$ErrorActionPreference = "Stop"
$origem = Join-Path $env:APPDATA "hub"
$destino = Join-Path $env:APPDATA "hub-dev"
if (-not (Test-Path $origem)) { throw "Pasta $origem não encontrada" }
if (Test-Path $destino) { throw "A pasta $destino já existe; apague ou renomeie antes de copiar de novo" }

Copy-Item (Join-Path $origem "config") (Join-Path $destino "config") -Recurse
New-Item -ItemType Directory -Force (Join-Path $destino "data") | Out-Null
Copy-Item (Join-Path $origem "data\hub.db") (Join-Path $destino "data\hub.db")
foreach ($extra in @("hub.db-wal", "hub.db-shm")) {
    $arquivo = Join-Path $origem "data\$extra"
    if (Test-Path $arquivo) { Copy-Item $arquivo (Join-Path $destino "data\$extra") }
}

$config = Join-Path $destino "config\config.toml"
if (Test-Path $config) {
    $texto = Get-Content $config -Raw
    $texto = [regex]::Replace($texto, '(?m)^porta = \d+', 'porta = 47822')
    [System.IO.File]::WriteAllText($config, $texto)
}

Write-Host "Dados copiados para $destino (sem a sessão do WhatsApp; vincule de novo pelo QR na versão dev)"
