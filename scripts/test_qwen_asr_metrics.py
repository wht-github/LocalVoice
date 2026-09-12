import unittest
from qwen_asr_metrics import aggregate, distance, score, units


class ScoringTests(unittest.TestCase):
    def test_mixed_word_boundaries(self):
        self.assertEqual(units('我用Rust，写 AI 工具。'), ['我', '用', 'rust', '写', 'ai', '工', '具'])

    def test_normalization(self):
        self.assertEqual(units("ＡＩ can't，go!", 'wer'), ['ai', 'cant', 'go'])

    def test_edits_and_empty_output(self):
        self.assertEqual(distance(list('abc'), list('adc!')), 2)
        self.assertEqual(score('你好 world', ''), {'errors': 3, 'reference_units': 3})

    def test_corpus_weighting(self):
        rows = [{'reference': 'a', 'text': ''}, {'reference': 'b c d', 'text': 'b c d'}]
        self.assertEqual(aggregate(rows, 'wer')['rate'], 0.25)

    def test_no_semantic_correction(self):
        self.assertGreater(score('十五', '15')['errors'], 0)


if __name__ == '__main__':
    unittest.main()
