"""Overlapping streams must not inflate reported GPU active wall time."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('analysis', Path(__file__).with_name('analyze-qwen-nsight.py'))
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class TimelineTests(unittest.TestCase):
    def test_empty_timeline(self):
        self.assertEqual(analysis.union_ns([]), 0)

    def test_overlapping_nested_and_unsorted_streams(self):
        self.assertEqual(analysis.union_ns([(8, 15), (0, 10), (2, 3)]), 15)

    def test_gaps_are_not_gpu_active_time(self):
        self.assertEqual(analysis.union_ns([(0, 10), (10, 12), (20, 25)]), 17)


if __name__ == '__main__':
    unittest.main()
