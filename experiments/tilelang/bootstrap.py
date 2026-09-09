"""Keep compiler caches local; import before torch/tilelang."""
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
os.environ.setdefault("TILELANG_CACHE_DIR", str(ROOT / ".runtime/tilelang-cache"))
os.environ.setdefault("TVM_FFI_CACHE_DIR", str(ROOT / ".runtime/tvm-ffi-cache"))
# NVRTC accepts ordinary torch tensors; the optional MSVC-built bridge is unnecessary.
os.environ.setdefault("TVM_FFI_DISABLE_TORCH_C_DLPACK", "1")
