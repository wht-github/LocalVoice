# 听写体验评测素材

这里没有新的真实语音基线，也没有合成的真人录音。

**默认质量门槛是不维护词表也能使用。** 先不传 `--terms`，分别测试原始识别、轻整理和笔记。常见计算机术语、英文名称和常用音译表达应由基础模型听对；个人词表只作为内部项目名、自定义缩写等专名的可选辅助，不是要求使用者日常逐词纠错的入口。

- `cases.text.jsonl`：8 条合成文本回归用例，另有 1 句用户提供的实际口述文本及其同源笔记检查用例。`source` 明确区分 `synthetic_text` 与 `user_supplied_text`。用于隔离文字整理效果，不能测 ASR 准确率、真实声学术语识别或说完至上屏延迟。
- `resource-cases.jsonl`：5 条额外的术语／引用边界短文本及 1 条 755 字符合成长笔记，供 `scripts/benchmark-cleanup-resources.py` 使用。与上述文本按正文去重后为 14 条短文本和 1 条长笔记；短文本分别测 clean/notes，长段只测 notes，共 29 个输出。检查项不传给模型，长段不属于真人录音或冻结验收集。
- `specialist-protocols.json`：AgenticASR-Refiner 与 MyVoiceTyping 作者原生提示、控制前缀及来源 revision，供 `scripts/benchmark-specialist-editors.py` 使用。同样 15 个输入分别跑原生与追加正文要求两组，每款 30 次生成；没有把检查项或期望术语传给模型。
- `agentic-author-chat_template.jinja`：原样保存的 [AgenticASR-Refiner 作者模板](https://huggingface.co/Andrew0425/AgenticASR-Refiner/blob/e41ca3654e2fc325f4e015b9127763d4b5ded5b8/chat_template.jinja)，来自 Apache-2.0 模型仓库 revision `e41ca3654e2fc325f4e015b9127763d4b5ded5b8`。用于对比 GGUF 内嵌模板；原生协议探针另核对 BOS 是否由运行时 tokenizer 添加，不更改模型模板。
- `agentic-fresh-cases.jsonl`：有限调优时在生成前固定的 8 条新中英混合短文本和 2 条长笔记（838／886 字符），均为合成口述风格文本；覆盖小数、范围、单位、日期、连续改口、撤回、条件和引用。仅由专用模型基准的 `--cases-file` 读取，检查项和术语不传给模型。单列原开发集的 `text-numbers-02` 回归后，两组共 22 个输出，仍只有 10 个新输入。
- `agentic-editing-instruction.txt`：本轮唯一候选追加提示。它没有带来值得采用的收益，保留为实验复现，应用不读取或使用它；不再基于这批结果反复调提示。
- `terms.example.json`：额外验证词表开关和术语后处理的开发用示例，默认评测不用它。`memory bandwidth`、`batch size` 等刻意留在词表外，以观察未收录内容是否被误改。示例收录的常见技术词不代表用户必须维护它们；词表不是参考答案，不能自动从 `expected_terms` 生成。
- `manifest.audio.example.jsonl`：真实录音的清单模板，音频路径是占位符，不能直接运行。将它复制到本地 `outputs/dictation-data/`，放入本人主动提供的录音，重写逐字参考及术语标注。

用户原话保留为：

> 你把那个AGV的blueprint给我导入UE，然后把空间的坐标标一下，跑一下transition，看看运动变化

词表还包含用户提出的 `model`、`Unreal`、`shader` 等术语。用户原句中没有出现它们，因此不将它们计入该句的期望术语。两个 AGV 用例只是对同一文本的不同审阅重点，不是两个独立质量样本。当前全部为开发集，尚未建立冻结验收集。

这些词只是使用场景的例子。后续真人录音必须覆盖更广的常见计算机词汇，包含未在词表、固定提示或开发样例中预登记的术语，以及自然口述中新出现的表达；不能只测上述 7 个词或将它们预先提示给模型来证明质量。录制时保留本人日常语速与发音，包括中文句子夹英文名称、字母缩写、带中文口音的英语以及短句之间自然切换中英文。常用音译与歧义词的允许写法应在查看输出前人工确认。

`expected_terms` 是隐藏于模型的人工评测标注，空词表时仍可用于计分。真实音频、逐字参考和冻结集尚待使用者主动提供，不能用合成文本或生成的录音替代真实识别成绩。新素材中的普通词、数字、否定和自我修正也要一起检查，避免为命中术语而损坏原意。

运行、计分口径及人工审阅见 [听写评测说明](../../../docs/dictation-evaluation.md)。
