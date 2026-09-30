# Builds Tesseract and Leptonica for the OCR POC of ADR 0015 (option C, #99), locally and in CI.
#
# Downloads the pinned sources and language data (each checked against its SHA-256), then builds
# static libraries with CMake and Visual Studio: without network (curl), archive, image-format or
# GUI support, and without the legacy engine. Everything goes under -Out (default target/ocr-poc,
# which git ignores); nothing is installed on the machine. The POC only: whether Tesseract ships,
# and how it would be built, is for ADR 0015 to decide.
#
# Usage, from the repository root: powershell -File scripts/ocr-poc/build-tesseract.ps1
param([string]$Out = "target/ocr-poc")
$ErrorActionPreference = "Stop"

$out = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Out))
$downloads = Join-Path $out "downloads"
$install = Join-Path $out "install"
New-Item -ItemType Directory -Force $downloads | Out-Null

# The language data comes from one commit of tessdata_fast (the smaller "fast" LSTM models).
$tessdata = "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd"
$files = @(
    @{
        Name = "tesseract-5.5.3.tar.gz"
        Url = "https://github.com/tesseract-ocr/tesseract/archive/refs/tags/5.5.3.tar.gz"
        Sha256 = "9218e62793116d42a9f6d14cd9348518b27f382096eea3d0f2d1a24616bb5884"
    },
    @{
        Name = "leptonica-1.87.0.tar.gz"
        Url = "https://github.com/DanBloomberg/leptonica/releases/download/1.87.0/leptonica-1.87.0.tar.gz"
        Sha256 = "c73363397f96eb1295602bf44d708a994ad42046c791bf03ea0505d829bdb6a7"
    },
    @{
        Name = "eng.traineddata"
        Url = "$tessdata/eng.traineddata"
        Sha256 = "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2"
    },
    @{
        Name = "chi_tra.traineddata"
        Url = "$tessdata/chi_tra.traineddata"
        Sha256 = "529c5b5797d64b126065cd55f2bb4c7fd7b15790798091b1ff259941a829330b"
    }
)
foreach ($file in $files) {
    $path = Join-Path $downloads $file.Name
    if (-not (Test-Path $path)) {
        Write-Host "download $($file.Url)"
        Invoke-WebRequest -Uri $file.Url -OutFile $path -UseBasicParsing
    }
    $hash = (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant()
    if ($hash -ne $file.Sha256) {
        Remove-Item $path
        throw "$($file.Name): SHA-256 $hash, expected $($file.Sha256)"
    }
}

foreach ($archive in "tesseract-5.5.3.tar.gz", "leptonica-1.87.0.tar.gz") {
    tar -xzf (Join-Path $downloads $archive) -C $out
    if ($LASTEXITCODE -ne 0) { throw "could not extract $archive" }
}

function Build($name, $source, [string[]]$options) {
    $build = Join-Path $out "build/$name"
    # No -G: CMake takes the newest Visual Studio it finds (Build Tools 2019 here, 2022 on CI).
    cmake -S $source -B $build -A x64 `
        "-DCMAKE_INSTALL_PREFIX=$install" `
        "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL" `
        -DBUILD_SHARED_LIBS=OFF -DSW_BUILD=OFF @options
    if ($LASTEXITCODE -ne 0) { throw "cmake could not configure $name" }
    cmake --build $build --config Release --target install --parallel
    if ($LASTEXITCODE -ne 0) { throw "cmake could not build $name" }
}

# Leptonica: only its image structures; pages arrive as raw pixels, so no image format libraries.
Build "leptonica" (Join-Path $out "leptonica-1.87.0") @(
    "-DBUILD_PROG=OFF",
    "-DENABLE_ZLIB=OFF", "-DENABLE_PNG=OFF", "-DENABLE_GIF=OFF", "-DENABLE_JPEG=OFF",
    "-DENABLE_TIFF=OFF", "-DENABLE_WEBP=OFF", "-DENABLE_OPENJPEG=OFF"
)
# Tesseract: the LSTM engine only; no curl (no network), no libarchive, no ScrollView (no GUI).
Build "tesseract" (Join-Path $out "tesseract-5.5.3") @(
    "-DLeptonica_DIR=$install/lib/cmake/leptonica",
    "-DBUILD_TRAINING_TOOLS=OFF", "-DBUILD_TESTS=OFF", "-DGRAPHICS_DISABLED=ON",
    "-DDISABLED_LEGACY_ENGINE=ON", "-DDISABLE_CURL=ON", "-DDISABLE_ARCHIVE=ON",
    "-DDISABLE_TIFF=ON", "-DOPENMP_BUILD=OFF", "-DENABLE_NATIVE=OFF", "-DINSTALL_CONFIGS=OFF"
)

Copy-Item (Join-Path $downloads "*.traineddata") (New-Item -ItemType Directory -Force (Join-Path $install "tessdata"))
Write-Host "installed into $install"
Get-ChildItem (Join-Path $install "lib") -Filter *.lib | ForEach-Object { Write-Host "  $($_.Name) $($_.Length)" }
