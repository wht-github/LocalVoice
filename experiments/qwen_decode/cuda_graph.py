"""Experimental batch-one greedy Qwen3-ASR decode graph, PyTorch CUDA only.

Audio encoding and prefill stay in the original model. This is intentionally not
a replacement for Transformers' general generation API (sampling/beams/processors).
"""
import torch
from transformers import StaticCache


class GreedyDecodeGraph:
    @torch.inference_mode()
    def __init__(self, model, capacity=2048):
        self.model = model
        self.capacity = capacity
        self.cache = StaticCache(config=model.config.text_config, max_cache_len=capacity)
        self.token = torch.zeros((1, 1), dtype=torch.long, device=model.device)
        self.position = torch.zeros_like(self.token)
        self.indices = torch.arange(capacity, device=model.device).view(1, 1, 1, -1)
        self.graph = torch.cuda.CUDAGraph()
        # Initialize all lazy cache allocations and CUDA libraries before capture.
        stream = torch.cuda.Stream()
        stream.wait_stream(torch.cuda.current_stream())
        with torch.cuda.stream(stream):
            for _ in range(3):
                self._step()
        torch.cuda.current_stream().wait_stream(stream)
        torch.cuda.synchronize()
        with torch.cuda.graph(self.graph, stream=stream):
            self._step()
        torch.cuda.synchronize()

    def _step(self):
        mask = self.indices <= self.position.view(1, 1, 1, 1)
        hidden = self.model.model.language_model(
            input_ids=self.token, position_ids=self.position,
            attention_mask={'full_attention': mask}, past_key_values=self.cache,
            use_cache=True).last_hidden_state
        next_token = self.model.lm_head(hidden).argmax(dim=-1)
        # These updates are captured too: host only replays and reads the token.
        self.token.copy_(next_token)
        self.position.add_(1)

    @torch.inference_mode()
    def generate(self, inputs, max_new_tokens=1024):
        ids = inputs['input_ids']
        if ids.shape[0] != 1 or ids.shape[1]+max_new_tokens > self.capacity:
            raise ValueError('Graph requires batch=1 and prompt+token budget <= cache capacity')
        if max_new_tokens < 1:
            raise ValueError('max_new_tokens must be positive')
        if 'attention_mask' in inputs and not bool(inputs['attention_mask'].all()):
            raise ValueError('Padded batches are not supported by this experiment')
        eos = self.model.generation_config.eos_token_id
        eos = set(eos if isinstance(eos, list) else [eos])
        # Always build the request cache from scratch; no state leaks between recordings.
        prefill = self.model(**inputs, use_cache=True, logits_to_keep=1)
        first = prefill.logits[:, -1:].argmax(dim=-1)
        generated = [int(first.item())]
        if generated[0] in eos:
            return torch.tensor([generated], device=ids.device)
        n = ids.shape[1]
        for target, source in zip(self.cache.layers, prefill.past_key_values.layers, strict=True):
            target.keys[:, :, :n].copy_(source.keys)
            target.values[:, :, :n].copy_(source.values)
            target.cumulative_length.fill_(n)
        self.token.copy_(first)
        self.position.fill_(n)
        del prefill
        for _ in range(max_new_tokens-1):
            self.graph.replay()
            token = int(self.token.item())
            generated.append(token)
            if token in eos:
                break
        return torch.tensor([generated], device=ids.device)
