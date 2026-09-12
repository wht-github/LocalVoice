# Generated evaluation summary

Mean latency = mean of six per-recording medians (three warm measurements each).
VRAM = device-wide sampled peak, including other applications. Different splits use different recordings.

| Split | Configuration | Overall MER | Latency (s) | Decode (ms/token) | Device peak (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| pilot | bf16 | 9.007% | 0.506 | 24.88 | 6026 |
| pilot | q8 | 9.007% | 0.284 | 12.34 | 4489 |
| pilot | q6 | 9.238% | 0.251 | 10.29 | 4289 |
| pilot | q4 | 9.469% | 0.231 | 8.46 | 4047 |
| pilot | q8-no-graphs | 9.007% | 0.334 | 15.04 | 4602 |
| pilot | q8-no-fa | 9.007% | 0.306 | 13.47 | 4782 |
| holdout | bf16 | 9.821% | 0.492 | 22.70 | 6082 |
| holdout | q8 | 9.821% | 0.300 | 12.82 | 4447 |
| holdout | q4 | 9.821% | 0.211 | 7.98 | 3693 |
