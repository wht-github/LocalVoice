#!/usr/bin/env bash
# Source this file from WSL. Runtime and models stay on the Linux filesystem.
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export VOICE_RUNTIME="${VOICE_RUNTIME:-$HOME/.local/share/local-voice-app}"
export HF_HOME="$VOICE_RUNTIME/huggingface"
export HF_ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"
export MODELSCOPE_CACHE="$VOICE_RUNTIME/modelscope"
export UV_CACHE_DIR="$VOICE_RUNTIME/uv-cache"
export UV_PYTHON_INSTALL_DIR="$VOICE_RUNTIME/python"
export HF_HUB_DISABLE_TELEMETRY=1
export HF_HUB_DISABLE_XET=1
export VLLM_NO_USAGE_STATS=1
export DO_NOT_TRACK=1
export TOKENIZERS_PARALLELISM=false
export OMP_NUM_THREADS=4
export UV_HTTP_TIMEOUT=120
export UV_DEFAULT_INDEX="${UV_DEFAULT_INDEX:-https://pypi.tuna.tsinghua.edu.cn/simple}"
mkdir -p "$VOICE_RUNTIME" "$PROJECT_DIR/.runtime" "$PROJECT_DIR/outputs"
