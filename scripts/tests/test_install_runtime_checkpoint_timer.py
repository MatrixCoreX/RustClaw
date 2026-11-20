import sys
from pathlib import Path
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import install_runtime_checkpoint_timer as timer


class TimerTests(unittest.TestCase):
    def test_daily_and_retry_use_same_operator_and_durable_coordinator(self):
        service,daily=timer.units(Path('/tmp/space and %/runtime'),Path('/backup space'),'ordinary-user','/usr/bin/python3','backup-host','/private/backups',vault_key_file=Path('/private/recovery.key'))
        retry,scheduled=timer.retry_units(service)
        self.assertIn('User=ordinary-user',service)
        self.assertEqual(service.count('ExecStart='),1)
        self.assertIn('runtime_backup_schedule.py',service)
        self.assertIn('space and %%/runtime',service)
        self.assertIn('--vault-key-file',service)
        self.assertIn('ExecStopPost=',service)
        self.assertIn(' --retry-only',retry)
        self.assertIn('OnUnitActiveSec=15min',scheduled)
        self.assertIn('OnCalendar=*-*-* 04:30:00',daily)
        self.assertNotIn('runtime_checkpoint_remote.py',service)
    def test_unsafe_unit_values_invalid_scopes_and_incomplete_targets_fail(self):
        with self.assertRaises(RuntimeError):timer.quoted('path\nExecStart=bad')
        for kw in ({'scopes':['unknown']},{'retain_versions':0},{'retry_limit':99},{'vault_key_file':Path('/key')}):
            with self.assertRaises(RuntimeError):timer.units(Path('/root'),Path('/backup'),'user','python3',**kw)
    def test_unsupported_platform_never_runs_linux_commands(self):
        with patch.object(timer.sys,'platform','darwin'),patch.object(timer.sys,'argv',['install','--root','/tmp/root','--destination','/tmp/backups']),patch.object(timer.subprocess,'run') as run:
            self.assertEqual(timer.main(),1);run.assert_not_called()


if __name__=='__main__':unittest.main()
