param(
    [ValidateRange(1,32)][int]$Jobs = 4,
    [string]$CudaArchitectures = '89',
    [switch]$ConfigureOnly,
    [switch]$VerifyCuda
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'enter-llama-env.ps1')
$source = Join-Path $llamaRoot 'external/llama.cpp'
$build = Join-Path $llamaRoot '.runtime/llama-build'
$cmake = Join-Path $llamaVenv 'Scripts/cmake.exe'
if (!(Test-Path -LiteralPath "$source/CMakeLists.txt")) { throw 'Run scripts/setup-llama-native.ps1 first.' }
$buildTests = if ($VerifyCuda) { 'ON' } else { 'OFF' }
& $cmake -S $source -B $build -G Ninja "-DCMAKE_MAKE_PROGRAM=$llamaVenv/Scripts/ninja.exe" `
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=cl -DCMAKE_CXX_COMPILER=cl `
    "-DCMAKE_CUDA_COMPILER=$env:CUDACXX" "-DCUDAToolkit_ROOT=$llamaCuda" `
    "-DCMAKE_CUDA_ARCHITECTURES=$CudaArchitectures" -DGGML_CUDA=ON `
    "-DLLAMA_BUILD_TESTS=$buildTests" -DLLAMA_OPENSSL=OFF
if ($LASTEXITCODE -ne 0) { throw 'CMake configuration failed.' }
if ($ConfigureOnly) { return }
& $cmake --build $build --parallel $Jobs --target llama-cli llama-server llama-quantize llama-bench llama-mtmd-cli
if ($LASTEXITCODE -ne 0) { throw 'llama.cpp build failed.' }
$devices = & "$build/bin/llama-cli.exe" --list-devices 2>&1
if ($LASTEXITCODE -ne 0) { throw 'CUDA backend device check failed.' }
$devices | Tee-Object "$build/devices.txt" | Write-Host
if (($devices -join "`n") -notmatch 'CUDA\d+:') { throw 'No CUDA device was registered by llama.cpp.' }
if ($VerifyCuda) {
    & $cmake --build $build --parallel $Jobs --target test-backend-ops
    if ($LASTEXITCODE -ne 0) { throw 'Backend test build failed.' }
    & "$build/bin/test-backend-ops.exe" test -b CUDA0 -o MUL_MAT `
        -p 'type_a=(f16|q8_0),type_b=f32,m=16,n=[1-9],k=256,' 2>&1 |
        Tee-Object "$build/cuda-verification.txt"
    if ($LASTEXITCODE -ne 0) { throw 'CUDA matrix multiplication verification failed.' }
    if (!(Select-String -Path "$build/cuda-verification.txt" -Pattern 'MUL_MAT\(.*\):.*OK' -Quiet)) {
        throw 'No matrix multiplication test actually ran successfully; check the test filter.'
    }
}
& git -C $source rev-parse HEAD | Set-Content "$build/source-commit.txt"
Write-Host "Build complete: $build/bin"
