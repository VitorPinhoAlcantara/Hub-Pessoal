param(
    [string]$Versao,
    [string]$Notas = "",
    [string]$Pasta = (Join-Path $env:USERPROFILE "HubPessoal\atualizacoes")
)

$ErrorActionPreference = "Stop"
$raiz = Split-Path -Parent $PSScriptRoot
Set-Location $raiz

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "O GitHub CLI (gh) não está instalado" }
if (git status --porcelain) { throw "Há alterações sem commit. Faça o commit antes de publicar." }
if ((git rev-parse --abbrev-ref HEAD) -ne "main") { throw "Publique a partir da branch main" }

$cargoToml = Join-Path $raiz "Cargo.toml"
$texto = Get-Content $cargoToml -Raw
$atual = [regex]::Match($texto, '(?m)^version = "(\d+)\.(\d+)\.(\d+)"')
if (-not $atual.Success) { throw "Versão não encontrada em Cargo.toml" }
if (-not $Versao) {
    $Versao = "{0}.{1}.{2}" -f $atual.Groups[1].Value, $atual.Groups[2].Value, ([int]$atual.Groups[3].Value + 1)
}
$tag = "v$Versao"
if (git tag --list $tag) { throw "A tag $tag já existe" }

$anterior = git describe --tags --abbrev=0 2>$null
$intervalo = if ($anterior) { "$anterior..HEAD" } else { "HEAD" }
$mudancas = git log $intervalo --no-merges --pretty=format:"- %s" | Where-Object { $_ -notmatch '^- chore: versão' }
if (-not $mudancas) { $mudancas = @("- Ajustes internos") }
$resumo = if ($Notas) { $Notas } else { $mudancas -join "`n" }

$texto = $texto.Remove($atual.Index, $atual.Length).Insert($atual.Index, "version = `"$Versao`"")
[System.IO.File]::WriteAllText($cargoToml, $texto)
Write-Host "Publicando a versão $Versao"

$destino = Join-Path $raiz "target\distribuicao\release"
New-Item -ItemType Directory -Force $destino | Out-Null

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
$arquivoInstalador = Join-Path $Pasta $nome
Copy-Item $instalador.FullName $arquivoInstalador -Force
$hash = (Get-FileHash $arquivoInstalador -Algorithm SHA256).Hash.ToLower()

$manifesto = [ordered]@{
    versao       = $Versao
    instalador   = $nome
    sha256       = $hash
    notas        = $resumo
    publicado_em = (Get-Date -Format "yyyy-MM-ddTHH:mm:ss")
}
$json = $manifesto | ConvertTo-Json
[System.IO.File]::WriteAllText((Join-Path $Pasta "ultima.json"), $json)
$manifestoRelease = Join-Path $destino "HubPessoal-atualizacao.json"
[System.IO.File]::WriteAllText($manifestoRelease, $json)

Get-ChildItem $Pasta -Filter "HubPessoal-*-setup.exe" | Sort-Object LastWriteTime -Descending | Select-Object -Skip 3 | Remove-Item -Force

Write-Host "Pronto: $nome em $Pasta"

$corpo = @"
## Instalação

Baixe o arquivo $nome e execute. Quem já tem o Hub Pessoal instalado recebe esta versão pelo próprio app.

## Mudanças

$($mudancas -join "`n")

## Arquivos

- ${nome}: instalador para Windows.
- HubPessoal-atualizacao.json: usado pelo atualizador do app para conferir a versão e o SHA-256 do instalador. Não precisa ser baixado.
"@
if ($Notas) { $corpo = "$Notas`n`n$corpo" }
$arquivoCorpo = Join-Path $destino "notas-da-release.md"
[System.IO.File]::WriteAllText($arquivoCorpo, $corpo)

Write-Host ""
Write-Host $corpo
Write-Host ""
$resposta = Read-Host "Fazer o commit da versão $Versao, enviar para a main e criar a release $tag? (s/n)"
if ($resposta -ne "s") {
    Write-Host "Nada foi enviado. A versão $Versao ficou só no Cargo.toml e no Cargo.lock."
    return
}

git add Cargo.toml Cargo.lock
git commit -m "chore: versão $Versao"
if ($LASTEXITCODE -ne 0) { throw "Falha no commit da versão" }
git push origin main
if ($LASTEXITCODE -ne 0) { throw "Falha ao enviar para a main" }

gh release create $tag $arquivoInstalador $manifestoRelease --target main --title "Hub Pessoal $Versao" --notes-file $arquivoCorpo
if ($LASTEXITCODE -ne 0) { throw "Falha ao criar a release" }
Write-Host "Release $tag publicada"
