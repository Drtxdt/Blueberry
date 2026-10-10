import copy
import importlib.util
from pathlib import Path
import unittest
import tempfile


spec = importlib.util.spec_from_file_location('comparison_collector', Path(__file__).parents[1] / 'scripts/run-comparison.py')
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)


def report():
    return {'modes': {mode: {
        'startup': [100.0, 200.0],
        'hot': {scenario: {cache: {
            'input_echo': [1.0, 2.0, 100.0],
            'menu': None if mode == 'plain' else [2.0, 3.0, 200.0],
        } for cache in ('cache_miss', 'cache_hit')} for scenario in collector.CASES}
    } for mode in collector.MODES}}


class ComparisonCollectorTests(unittest.TestCase):
    def capability_probes(self, passed):
        return [(Path(str(index)), {'passed': passed, 'queries': [{
            'input_echo': 25.0, 'menu': 30.0 if passed else None,
            'failed': not passed, 'elapsed_ms': 20001.0,
        }]}) for index in range(3)]

    def test_capability_timeout_requires_valid_input(self):
        probes = self.capability_probes(False)
        self.assertTrue(collector.capability_unavailable('js', probes))
        for invalid in (None, float('nan'), -1, True):
            with self.subTest(invalid=invalid):
                probes[0][1]['queries'][0]['input_echo'] = invalid
                with self.assertRaisesRegex(AssertionError, 'invalid input echo'):
                    collector.capability_unavailable('js', probes)

    def test_capability_success_requires_a_menu_observation(self):
        probes = self.capability_probes(True)
        self.assertFalse(collector.capability_unavailable('js', probes))
        probes[0][1]['queries'][0]['menu'] = None
        with self.assertRaisesRegex(AssertionError, 'invalid menu observation'):
            collector.capability_unavailable('js', probes)

    def test_capability_mixed_results_remain_a_failure(self):
        probes = self.capability_probes(False)
        probes[0] = self.capability_probes(True)[0]
        with self.assertRaisesRegex(AssertionError, 'inconsistent capability'):
            collector.capability_unavailable('js', probes)

    def test_capability_short_or_failed_collection_is_not_absence(self):
        probes = self.capability_probes(False)
        with self.assertRaisesRegex(AssertionError, 'insufficient capability'):
            collector.capability_unavailable('js', probes[:2])
        probes[0][1]['queries'][0]['elapsed_ms'] = 100.0
        with self.assertRaisesRegex(AssertionError, 'collection failure'):
            collector.capability_unavailable('js', probes)

    def test_profile_creation_removal_and_edits_invalidate_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'profile.ps1'
            absent = [{'path': str(path), 'sha256': None}]
            self.assertTrue(collector.profiles_unchanged(absent))
            path.write_text('original', encoding='utf-8')
            self.assertFalse(collector.profiles_unchanged(absent))
            frozen = [{'path': str(path), 'sha256': collector.sha(path)}]
            self.assertTrue(collector.profiles_unchanged(frozen))
            path.write_text('changed', encoding='utf-8')
            self.assertFalse(collector.profiles_unchanged(frozen))
            path.unlink()
            self.assertFalse(collector.profiles_unchanged(frozen))

    def test_keeps_slow_samples_and_recomputes_statistics(self):
        value = report()
        collector.summarize_measurements(value, 2, 3)
        measured = value['modes']['candidate']['hot']['root']['cache_miss']['menu']
        self.assertEqual(measured['samples'], [2.0, 3.0, 200.0])
        self.assertEqual(measured['median'], 3.0)
        self.assertEqual(measured['p95'], 200.0)

    def test_rejects_short_samples_and_missing_matrix_groups(self):
        baseline = report()
        mutations = [
            lambda r: r['modes'].pop('beta6'),
            lambda r: r['modes']['candidate']['startup'].pop(),
            lambda r: r['modes']['candidate']['hot'].pop('fuzzy'),
            lambda r: r['modes']['candidate']['hot']['git'].pop('cache_hit'),
            lambda r: r['modes']['candidate']['hot']['root']['cache_miss']['input_echo'].pop(),
            lambda r: r['modes']['candidate']['hot']['root']['cache_miss']['menu'].pop(),
        ]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                value = copy.deepcopy(baseline)
                mutate(value)
                with self.assertRaises(AssertionError):
                    collector.summarize_measurements(value, 2, 3)

    def test_rejects_nonfinite_negative_and_nonnumeric_samples(self):
        for invalid in (float('nan'), float('inf'), -1.0, True, '1', None):
            with self.subTest(invalid=invalid):
                value = report()
                value['modes']['candidate']['hot']['path']['cache_hit']['menu'][0] = invalid
                with self.assertRaisesRegex(AssertionError, 'Invalid latency'):
                    collector.summarize_measurements(value, 2, 3)

    def test_missing_comparator_capability_needs_retained_probes(self):
        value = report()
        measurement = value['modes']['inshellisense']['hot']['root']['cache_miss']
        measurement.update(menu=None, menu_status='not_observed_in_capability_probe', capability_probes=[{'report': str(i)} for i in range(3)])
        valid = copy.deepcopy(value)
        collector.summarize_measurements(valid, 2, 3)
        self.assertIsNone(valid['modes']['inshellisense']['hot']['root']['cache_miss']['menu'])
        measurement['capability_probes'].pop()
        with self.assertRaisesRegex(AssertionError, 'unexplained missing menu'):
            collector.summarize_measurements(value, 2, 3)


if __name__ == '__main__':
    unittest.main()
