import copy
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location('resources', Path(__file__).parents[1] / 'scripts/resource-statistics.py')
RESOURCES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RESOURCES)


class ResourceStatisticsTests(unittest.TestCase):
    def setUp(self):
        self.report = dict(schema=2, measurement_complete=True, collection_errors=[], qpc_frequency=1000,
                           elapsed_seconds=30, root_pid=10, root_started_utc='created',
                           samples=[dict(qpc=1000 + second*1000, processes=[dict(pid=10, started_utc='created',
                                cpu_seconds=second/1000, working_set_bytes=1024*(second+1), private_bytes=512)])
                                for second in range(31)], deltas=[dict(pid=10, cpu_percent_one_core=.1)])

    def test_recomputes_cpu_and_retains_all_memory_snapshots(self):
        result = RESOURCES.summarize(self.report)
        self.assertAlmostEqual(result['cpu_percent_one_core'], .1)
        self.assertEqual(result['working_set_bytes']['peak'], 31744)
        self.assertEqual(len(result['working_set_bytes']['samples']), 31)

    def test_invalid_or_changed_tree_cannot_be_a_zero_cpu_sample(self):
        for change in ('missing', 'recycled', 'counter', 'nonfinite', 'short', 'delta', 'duplicate', 'incomplete'):
            with self.subTest(change=change):
                report = copy.deepcopy(self.report)
                if change == 'missing': report['samples'][5]['processes'] = []
                if change == 'recycled': report['samples'][5]['processes'][0]['started_utc'] = 'other'
                if change == 'counter': report['samples'][5]['processes'][0]['cpu_seconds'] = 0
                if change == 'nonfinite': report['samples'][5]['processes'][0]['cpu_seconds'] = float('nan')
                if change == 'short': report['samples'] = report['samples'][:-1]
                if change == 'delta': report['deltas'][0]['cpu_percent_one_core'] = 0
                if change == 'duplicate': report['samples'][5]['processes'] *= 2
                if change == 'incomplete': report['measurement_complete'] = False
                with self.assertRaises(AssertionError): RESOURCES.summarize(report)


if __name__ == '__main__':
    unittest.main()
