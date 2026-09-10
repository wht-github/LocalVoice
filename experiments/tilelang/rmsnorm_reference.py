"""Qwen3RMSNorm.forward from installed Transformers 5.16.1, functional form.

Preserve the FP32 reduction and FP16 conversion BEFORE multiplying the weight.
No torch.compile, CUDA Graph, or other optimization is applied to this baseline.
"""

import torch


def rmsnorm_reference(x, weight, eps=1e-6):
    input_dtype = x.dtype
    hidden_states = x.to(torch.float32)
    variance = hidden_states.pow(2).mean(-1, keepdim=True)
    hidden_states = hidden_states * torch.rsqrt(variance + eps)
    return weight * hidden_states.to(input_dtype)
