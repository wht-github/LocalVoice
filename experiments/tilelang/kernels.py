"""Runnable baseline and a small exercise. Both write into caller-owned output."""
import bootstrap  # noqa: F401
import tilelang
import tilelang.language as T


@tilelang.jit(execution_backend="nvrtc")
def vector_add(A, B, C, block: int = 256):
    N = T.const("N")
    A: T.Tensor((N,), T.float32)
    B: T.Tensor((N,), T.float32)
    C: T.Tensor((N,), T.float32)
    with T.Kernel(T.ceildiv(N, block), threads=128) as bx:
        for i in T.Parallel(block):
            idx = bx * block + i
            if idx < N:
                C[idx] = A[idx] + B[idx]


@tilelang.jit(execution_backend="nvrtc")
def add_relu_exercise(A, B, C, block: int = 256):
    N = T.const("N")
    A: T.Tensor((N,), T.float32)
    B: T.Tensor((N,), T.float32)
    C: T.Tensor((N,), T.float32)
    with T.Kernel(T.ceildiv(N, block), threads=128) as bx:
        for i in T.Parallel(block):
            idx = bx * block + i
            if idx < N:
                # TODO: change this line to max(A[idx] + B[idx], 0).
                # Hint: TileLang provides T.max and T.float32(0).
                C[idx] = A[idx] + B[idx]
