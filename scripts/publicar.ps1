param(
    [string]$Versao,
    [string]$Notas = "",
    [string]$Pasta = (Join-Path $env:USERPROFILE "HubPessoal\atualizacoes")
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
Set-Location $raiz

$cargoToml = Join-Path $raiz "Cargo.toml"
$texto = Get-Content $cargoToml -Raw
$atual = [regex]::Match($texto, '(?m)^version = "(\d+)\.(\d+)\.(\d+)"')
if (-not $atual.Success) { throw "Versão não encontrada em Cargo.toml" }
if (-not $Versao) {
    $Versao = "{0}.{1}.{2}" -f $atual.Groups[1].Value, $atual.Groups[2].Value, ([int]$atual.Groups[3].Value + 1)
}
$texto = $texto.Remove($atual.Index, $atual.Length).Insert($atual.Index, "version = `"$Versao`"")
[System.IO.File]::WriteAllText($cargoToml, $texto)
Write-Host "Publicando a versão $Versao"

$destino = Join-Path $raiz "target\distribuicao\release"
New-Item -ItemType Directory -Force $destino | Out-Null

Push-Location (Join-Path $raiz "sidecars\whatsapp-bridge")
go build -o (Join-Path $destino "whatsapp-bridge.exe") .
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "Falha ao compilar o sidecar" }
Pop-Location

cargo build --release -p hub-app --features distribuicao --target-dir target\distribuicao
if ($LASTEXITCODE -ne 0) { throw "Falha ao compilar o hub" }
Copy-Item (Join-Path $destino "hub.exe") (Join-Path $destino "HubPessoal.exe") -Force

$pacote = Join-Path $raiz "target\distribuicao\pacote"
if (Test-Path $pacote) { Remove-Item -Recurse -Force $pacote }
cargo packager --release -p hub-app
if ($LASTEXITCODE -ne 0) { throw "Falha ao gerar o instalador" }

$instalador = Get-ChildItem $pacote -Recurse -Filter "*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $instalador) { throw "Instalador não encontrado em $pacote" }

New-Item -ItemType Directory -Force $Pasta | Out-Null
$nome = "HubPessoal-$Versao-setup.exe"
Copy-Item $instalador.FullName (Join-Path $Pasta $nome) -Force
$hash = (Get-FileHash (Join-Path $Pasta $nome) -Algorithm SHA256).Hash.ToLower()

$manifesto = [ordered]@{
    versao       = $Versao
    instalador   = $nome
    sha256       = $hash
    notas        = $Notas
    publicado_em = (Get-Date -Format "yyyy-MM-ddTHH:mm:ss")
}
$json = $manifesto | ConvertTo-Json
[System.IO.File]::WriteAllText((Join-Path $Pasta "ultima.json"), $json)

Get-ChildItem $Pasta -Filter "HubPessoal-*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -Skip 3 | Remove-Item -Force

Write-Host "Pronto: $nome em $Pasta"
