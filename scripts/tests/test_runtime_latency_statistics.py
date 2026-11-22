import sys
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'nl_tests'))
import summarize_rollout_metrics as metrics
import check_runtime_regression_budget as budget

class LatencyTests(unittest.TestCase):
    def test_nearest_rank_retains_tail_and_handles_empty_samples_as_unknown(self):
        self.assertEqual(metrics.percentile_nearest_rank([1,2,3,4,1000],50),3)
        self.assertEqual(metrics.percentile_nearest_rank([1,2,3,4,1000],95),1000)
        self.assertIsNone(metrics.percentile_nearest_rank([],95))

    def test_invalid_samples_cannot_become_fast_zero_duration_results(self):
        for value in (True,-1,float('nan'),float('inf')):
            with self.assertRaises(ValueError):metrics.percentile_nearest_rank([value],95)
            self.assertIsNone(metrics.harness_wall_time_ms({'harness_metrics':{'wall_time_ms':value}}))

    def test_existing_budget_and_tail_gate_rejection_paths_remain_active(self):
        budget.self_test()

if __name__=='__main__':unittest.main()
