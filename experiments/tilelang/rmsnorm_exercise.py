"""Your exercise: one block computes RMSNorm for one 1024-element row."""
import bootstrap  # noqa: F401
import torch
import tilelang
import tilelang.language as T


@tilelang.jit(execution_backend="nvrtc")
def rmsnorm_kernel(X, W, Y):
    M = T.const("M")
    X: T.Tensor((M, 1024), T.float16)
    W: T.Tensor((1024,), T.float16)
    Y: T.Tensor((M, 1024), T.float16)
    with T.Kernel(M, threads=128) as row:
        # Optional scratch space for your solution:
        # squares = T.alloc_fragment((1024,), T.float32)
        # total = T.alloc_fragment((1,), T.float32)
        #
        # TODO 1: Convert each input to FP32 and compute its square.
        # TODO 2: Sum across the row, then divide by 1024 and add epsilon=1e-6.
        #         Hint: T.reduce_sum(squares, total, dim=0)
        # TODO 3: Multiply x by the reciprocal square root, convert to FP16,
        #         then multiply by W. Hint: T.rsqrt, T.cast.
        squares = T.alloc_fragment((1024,), T.float32)
        total = T.alloc_fragment((1,), T.float32)

        for col in T.Parallel(1024):
            value = T.cast(X[row, col], T.float32)
            squares[col] = value * value
            # Starter only: compiles, but intentionally omits normalization.
            # Y[row, col] = X[row, col] * W[col]
        
        T.reduce_sum(squares, total, dim=0)
        mean_square = total[0] / T.float32(1024)
        scale = T.rsqrt(mean_square + T.float32(1e-6))

        for col in T.Parallel(1024):
            value = T.cast(X[row, col], T.float32)
            normalized = value * scale
            normalized_fp16 = T.cast(normalized, T.float16)
            Y[row, col] = normalized_fp16 * W[col]

def rmsnorm_candidate(x, weight):
    """Same allocating interface as the Qwen reference; contiguous FP16 only."""
    if x.dtype != torch.float16 or weight.dtype != torch.float16:
        raise ValueError("This exercise requires FP16 input and weight")
    if not x.is_cuda or weight.device != x.device:
        raise ValueError("Input and weight must be on the same CUDA device")
    if x.shape[-1] != 1024 or weight.shape != (1024,):
        raise ValueError("This exercise fixes hidden width to 1024")
    if not x.is_contiguous() or not weight.is_contiguous():
        raise ValueError("This exercise requires contiguous tensors")
    out = torch.empty_like(x)
    rmsnorm_kernel(x.view(-1, 1024), weight, out.view(-1, 1024))
    return out
