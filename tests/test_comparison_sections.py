"""A complete merge must retain every sample and reject broken shard evidence."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('merge_sections', Path(__file__).parents[1]/'scripts/merge-comparison-sections.py')
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
C = M.C


class SectionTests(unittest.TestCase):
    def reports(self, root):
        dependency = root/'dependency.bin'
        dependency.write_bytes(b'frozen')
        inventory = [dict(path=str(dependency), sha256=C.sha(dependency))]
        entries = []
        cap_path = root/'capability/comparison.json'
        identity = {key: 'same' for key in M.IDENTITY}
        identity.update(formal=True, source_dirty=False, startup_pairs=30, hot_samples_per_mode=300)

        def section(name, kind, scenario=None, pair_start=0):
            folder = root/name
            folder.mkdir()
            C.write(folder/'frozen.json', inventory)
            C.write(folder/'fixtures.json', inventory)
            report = dict(identity, section=kind, section_complete=True, measurement_complete=False,
                          fixture_sha256=C.sha(folder/'fixtures.json'), frozen_sha256=C.sha(folder/'frozen.json'),
                          profile_inventory=[], modes={}, hot_session_orders={}, startup_orders=[], unavailable={})
            for mode in C.MODES:
                report['modes'][mode] = dict(entry={}, dependencies=[], sha256='same', transport='same', host_mode='same',
                    transport_degraded=False, startup=C.stats([1]*30) if kind=='startup' else [], hot={}, first_queries=[])
            if kind == 'hot':
                report.update(scenario=scenario, pair_start=pair_start, pair_count=15,
                              capability_evidence=dict(path=str(cap_path.resolve()), sha256=C.sha(cap_path)))
                report['hot_session_orders'][scenario] = {cache:[C.rotate(i) for i in range(pair_start,pair_start+15)]
                                                          for cache in ('cache_miss','cache_hit')}
                for mode in C.MODES:
                    report['modes'][mode]['hot'][scenario] = {
                        cache:dict(input_echo=C.stats([pair_start+1]*150), menu=None if mode=='plain' else C.stats([2]*150))
                        for cache in ('cache_miss','cache_hit')}
            path = folder/'comparison.json'
            if kind=='capability':
                report['capability_probes']={}
                for case in C.CASES:
                    evidence=[]
                    for i in range(3):
                        probe=folder/f'{case}-{i}.json'
                        C.write(probe, dict(passed=True,queries=[dict(input_echo=1,menu=2)]))
                        evidence.append(dict(report=probe.name,sha256=C.sha(probe)))
                    report['capability_probes'][case]=evidence
            C.write(path,report)
            entries.append((path,report))

        section('capability','capability')
        section('startup','startup')
        for case in C.CASES:
            for start in (0,15): section(f'{case}-{start}','hot',case,start)
        return entries

    def test_complete_merge_recomputes_all_raw_samples(self):
        with tempfile.TemporaryDirectory() as tmp:
            result=M.merge(self.reports(Path(tmp)))
            self.assertTrue(result['measurement_complete'])
            values=result['modes']['candidate']['hot']['root']['cache_hit']['input_echo']
            self.assertEqual(values['samples'],[1]*150+[16]*150)
            self.assertEqual(values['median'],8.5)
            self.assertEqual(values['p95'],16)

    def test_missing_overlap_identity_and_statistics_are_rejected(self):
        for mutation, message in (
            (lambda e:e.pop(), 'Incomplete hot scenario'),
            (lambda e:e[3][1].update(pair_start=0), 'overlapping'),
            (lambda e:e[3][1].update(probe_sha256='other'), 'identity mismatch'),
            (lambda e:e[3][1]['modes']['candidate']['hot']['root']['cache_hit']['input_echo'].update(p95=0), 'Incorrect shard statistic'),
            (lambda e:e[3][1].update(section_complete=False), 'Incomplete or unsharded'),
        ):
            with self.subTest(message=message), tempfile.TemporaryDirectory() as tmp:
                entries=self.reports(Path(tmp))
                mutation(entries)
                with self.assertRaisesRegex(AssertionError,message): M.merge(entries)

    def test_changed_dependency_is_not_a_valid_slow_sample(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp)
            entries=self.reports(root)
            (root/'dependency.bin').write_bytes(b'changed')
            with self.assertRaisesRegex(AssertionError,'Changed measurement dependency'): M.merge(entries)


if __name__=='__main__': unittest.main()
