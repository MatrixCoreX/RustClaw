import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import runtime_backup_capacity as capacity


class CapacityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.destination = self.root / 'backups'
        self.destination.mkdir()

    def usage(self, free):return shutil._ntuple_diskusage(10000, 10000-free, free)

    def test_reserve_and_estimate_share_admission_boundary_without_creating_files(self):
        with patch.object(capacity.shutil, 'disk_usage', return_value=self.usage(1000)):
            result = capacity.capacity_report(self.root, self.destination, 200)
        self.assertEqual(result['status'], 'insufficient')
        self.assertEqual(result['required_available_bytes'], capacity.DEFAULT_ESTIMATE + 200)
        self.assertEqual(result['destination_shortfall_bytes'], capacity.DEFAULT_ESTIMATE - 800)
        self.assertEqual(list(self.destination.iterdir()), [])

    def test_exact_threshold_is_ready_and_workspace_has_its_own_reserve(self):
        with patch.object(capacity, 'estimate_snapshot_bytes', return_value=500), patch.object(capacity.shutil, 'disk_usage', side_effect=[self.usage(700), self.usage(200)]):
            result = capacity.capacity_report(self.root, self.destination, 200)
        self.assertEqual(result['status'], 'ready')
        with patch.object(capacity, 'estimate_snapshot_bytes', return_value=500), patch.object(capacity.shutil, 'disk_usage', side_effect=[self.usage(700), self.usage(199)]):
            result = capacity.capacity_report(self.root, self.destination, 200)
        self.assertEqual(result['source_shortfall_bytes'], 1)
        self.assertEqual(result['status'], 'insufficient')

    def test_estimate_uses_latest_checkpoint_and_never_follows_payload_links(self):
        old = self.destination / 'checkpoint-001';old.mkdir();(old/'manifest.json').write_text('{}');(old/'payload').write_bytes(b'old')
        recent = self.destination / 'checkpoint-002';recent.mkdir();(recent/'manifest.json').write_text('{}');(recent/'payload').write_bytes(b'newer')
        (recent/'untrusted').symlink_to(old/'payload')
        self.assertEqual(capacity.estimate_snapshot_bytes(self.destination), 7)

    def test_probe_failures_preserve_unknown_values_and_do_not_report_zero(self):
        with patch.object(capacity.shutil, 'disk_usage', side_effect=OSError('fixture probe unavailable')):
            value = capacity.capacity_report(self.root, self.destination, 200)
        self.assertEqual(value['status'], 'unavailable')
        self.assertIsNone(value['destination_available_bytes'])

    def test_only_verified_private_status_can_claim_version_coverage(self):
        (self.root/'data').mkdir();checkpoint = self.destination/'checkpoint-1';checkpoint.mkdir()
        (checkpoint/'manifest.json').write_text(json.dumps({'source_revision':'a'*40}))
        status = self.root/'data/runtime-checkpoint-status.json'
        status.write_text(json.dumps({'status':'failed','checkpoint':str(checkpoint)}))
        self.assertIsNone(capacity.verified_source_revision(self.root,self.destination))
        status.write_text(json.dumps({'status':'verified_remote','checkpoint':str(checkpoint)}))
        self.assertEqual(capacity.verified_source_revision(self.root,self.destination),'a'*40)
        status.unlink();status.symlink_to(checkpoint/'manifest.json')
        self.assertIsNone(capacity.verified_source_revision(self.root,self.destination))

    def checkpoint_database(self, path='data/state.db', previous=100):
        recent=self.destination/'checkpoint-003';recent.mkdir()
        (recent/'manifest.json').write_text(json.dumps({'files':[{'kind':'sqlite','path':path,'raw_bytes':previous}]}))
        return recent

    def test_live_database_and_wal_growth_increase_estimate_without_lowering_reserve(self):
        source=self.root/'data/state.db';source.parent.mkdir();source.write_bytes(b'x'*150)
        source.with_name(source.name+'-wal').write_bytes(b'x'*50)
        self.checkpoint_database()
        with patch.object(capacity,'estimate_snapshot_bytes',return_value=500),patch.object(capacity.shutil,'disk_usage',return_value=self.usage(750)):
            value=capacity.capacity_report(self.root,self.destination,200)
        self.assertEqual(value['estimated_database_growth_bytes'],100)
        self.assertEqual(value['required_available_bytes'],800)
        self.assertEqual(value['reserve_bytes'],200)
        self.assertEqual(value['status'],'insufficient')

    def test_shrinking_database_does_not_reduce_last_measured_snapshot_estimate(self):
        source=self.root/'data/state.db';source.parent.mkdir();source.write_bytes(b'x'*20)
        self.checkpoint_database()
        self.assertEqual(capacity.database_growth_bytes(self.root,self.destination),0)

    def test_unsafe_manifest_source_path_cannot_be_used_for_capacity(self):
        self.checkpoint_database('../outside.db')
        value=capacity.capacity_report(self.root,self.destination,200)
        self.assertEqual(value['status'],'unavailable')
        self.assertIsNone(value['destination_available_bytes'])

    def test_post_backup_warning_is_distinct_from_the_configured_admission_floor(self):
        usage=shutil._ntuple_diskusage(100*1024**3,97*1024**3,3*1024**3)
        with patch.object(capacity,'estimate_snapshot_bytes',return_value=512*1024**2),patch.object(capacity.shutil,'disk_usage',return_value=usage):
            value=capacity.capacity_report(self.root,self.destination,2*1024**3)
        self.assertEqual(value['status'],'ready')
        self.assertEqual(value['projected_destination_available_bytes'],int(2.5*1024**3))
        self.assertEqual(value['projected_destination_pressure'],'warning')


if __name__ == '__main__':unittest.main()
