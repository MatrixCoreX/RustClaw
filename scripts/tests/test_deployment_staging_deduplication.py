import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import deduplicate_deployment_staging as dedup
class Deduplication(unittest.TestCase):
    def test_paths_content_and_unique_evidence_survive(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);stage=root/'.deploy-staging';stage.mkdir()
            for name in ['a','b']:(stage/name).write_bytes(b'x'*1024*1024)
            (stage/'evidence').write_text('keep')
            self.assertEqual(dedup.deduplicate(root)['deduplicated_bytes'],1024*1024)
            with patch.object(dedup,'require_idle'):
                dedup.deduplicate(root,True)
            self.assertEqual((stage/'a').stat().st_ino,(stage/'b').stat().st_ino)
            self.assertEqual((stage/'evidence').read_text(),'keep')
            self.assertEqual(dedup.deduplicate(root)['deduplicated_bytes'],0)
    def test_live_reference_failure_prevents_replacement(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);stage=root/'.deploy-staging';stage.mkdir()
            for name in ['a','b']:(stage/name).write_bytes(b'x'*1024*1024)
            with patch.object(dedup,'require_idle',side_effect=RuntimeError('busy')):
                with self.assertRaises(RuntimeError):dedup.deduplicate(root,True)
            self.assertNotEqual((stage/'a').stat().st_ino,(stage/'b').stat().st_ino)
