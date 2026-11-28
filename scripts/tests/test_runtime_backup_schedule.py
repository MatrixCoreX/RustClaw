import hashlib
import json
import tarfile
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import runtime_backup_schedule as schedule
from runtime_checkpoint_storage import write_json


class ScheduleTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.base=Path(self.tmp.name).resolve();self.root=self.base/'root';(self.root/'data').mkdir(parents=True)
        self.destination=self.base/'backups';self.destination.mkdir(mode=0o700)
        self.settings={'root':self.root,'destination':self.destination,'service_mode':'systemd-system','service_unit':'fixture.service','reserve_bytes':0,'read_fence_seconds':1800,'remote_host':'fixture-host','remote_directory':'/private/backups','vault_key_file':None,'scopes':['models'],'retain_versions':3,'retry_limit':3}
        self.now=1000
        self.clock=patch.object(schedule.time,'time',side_effect=lambda:self.now);self.clock.start();self.addCleanup(self.clock.stop)
        self.sample=patch.object(schedule,'sample_inventory');self.sample.start();self.addCleanup(self.sample.stop)
        self.revision=patch.object(schedule.checkpoint,'running_revision',return_value='a'*40);self.revision.start();self.addCleanup(self.revision.stop)
    def state(self):return json.loads((self.root/'data/runtime-backup-schedule.json').read_text())
    def create_unpublished(self,settings):
        path=self.destination/('checkpoint-'+str(self.now));path.mkdir(mode=0o700)
        control=path/'control-files.tar.xz'
        with tarfile.open(control,'w:xz'):pass
        write_json(path/'manifest.json',{'schema_version':1,'kind':'runtime_control_checkpoint','created_at_ts':self.now,'source_revision':'a'*40,'files':[],'scope':{},'runtime_pause_seconds':0,'control_archive':{'name':control.name,'sha256':hashlib.sha256(control.read_bytes()).hexdigest(),'bytes':control.stat().st_size}})
        return {'schema_version':1,'status':'verified','checkpoint':str(path),'created_at_ts':self.now}
    def create(self,settings):
        result=self.create_unpublished(settings)
        write_json(self.root/'data/runtime-checkpoint-status.json',result);return result
    def due(self):self.now=self.state()['next_retry_at_ts']+1
    def test_library_create_result_is_persisted_before_real_remote_status_binding(self):
        import runtime_checkpoint_remote
        previous={'schema_version':1,'status':'verified','checkpoint':str(self.destination/'checkpoint-retired'),'created_at_ts':self.now-1}
        write_json(self.root/'data/runtime-checkpoint-status.json',previous)
        with patch.object(schedule.checkpoint,'create',side_effect=lambda *args,**kwargs:self.create_unpublished(self.settings)) as create,patch.object(runtime_checkpoint_remote,'sync',return_value={'status':'verified'}):
            result=schedule.run_cycle(self.settings)
        self.assertEqual(result['status'],'verified');self.assertEqual(create.call_count,1)
        status=json.loads((self.root/'data/runtime-checkpoint-status.json').read_text())
        self.assertEqual(status['checkpoint'],self.state()['checkpoint']);self.assertEqual(status['status'],'verified_remote')
    def test_crash_after_directory_publication_before_status_or_journal_update_reuses_it(self):
        def interrupted(*args,**kwargs):
            self.create_unpublished(self.settings)
            raise KeyboardInterrupt('fixture crash before library return')
        with patch.object(schedule.checkpoint,'create',side_effect=interrupted):
            with self.assertRaises(KeyboardInterrupt):schedule.run_cycle(self.settings)
        self.assertEqual(self.state()['phase'],'local');self.assertFalse((self.root/'data/runtime-checkpoint-status.json').exists())
        with patch.object(schedule.checkpoint,'create') as create,patch.object(schedule,'send_remote',return_value={'status':'verified'}):
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'verified');create.assert_not_called()
        self.assertEqual(json.loads((self.root/'data/runtime-checkpoint-status.json').read_text())['checkpoint'],self.state()['checkpoint'])
    def test_remote_resume_repairs_retired_status_without_creating_or_restarting_local(self):
        import runtime_checkpoint_remote
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',side_effect=RuntimeError('vault_remote_receive_failed')):schedule.run_cycle(self.settings)
        state=self.state();self.due()
        write_json(self.root/'data/runtime-checkpoint-status.json',{'schema_version':1,'status':'verified','checkpoint':str(self.destination/'checkpoint-deleted'),'created_at_ts':1})
        with patch.object(schedule.checkpoint,'create') as create,patch.object(runtime_checkpoint_remote,'sync',return_value={'status':'verified'}):
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'verified');create.assert_not_called()
        self.assertEqual(json.loads((self.root/'data/runtime-checkpoint-status.json').read_text())['checkpoint'],state['checkpoint'])
    def test_remote_resume_never_replaces_a_newer_independent_checkpoint_status(self):
        import runtime_checkpoint_remote
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',side_effect=RuntimeError('vault_remote_receive_failed')):schedule.run_cycle(self.settings)
        self.now+=1;newer=self.create(self.settings);before=(self.root/'data/runtime-checkpoint-status.json').read_bytes();self.due()
        with patch.object(runtime_checkpoint_remote,'sync') as send:
            result=schedule.run_cycle(self.settings,retry_only=True);send.assert_not_called()
        self.assertEqual(result['error_code'],'backup_local_checkpoint_changed')
        self.assertEqual((self.root/'data/runtime-checkpoint-status.json').read_bytes(),before);self.assertTrue(Path(newer['checkpoint']).exists())
    def test_unverified_published_directory_is_not_adopted_as_success(self):
        self.create_unpublished(self.settings);(self.destination/'checkpoint-1000/control-files.tar.xz').write_bytes(b'corrupt fixture')
        with patch.object(schedule.checkpoint,'create') as create,patch.object(schedule,'send_remote') as send:
            result=schedule.run_cycle(self.settings)
        self.assertEqual(result['status'],'retry_scheduled');create.assert_not_called();send.assert_not_called()
        self.assertFalse((self.root/'data/runtime-checkpoint-status.json').exists())
    def test_remote_recovery_reuses_verified_local_without_stopping_service_again(self):
        with patch.object(schedule,'create_local',side_effect=self.create) as create,patch.object(schedule,'send_remote',side_effect=[RuntimeError('vault_remote_receive_failed'),{'status':'verified_encrypted'}]) as send:
            first=schedule.run_cycle(self.settings);self.assertEqual(first['status'],'retry_scheduled');self.assertEqual(create.call_count,1)
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'not_due')
            self.due();second=schedule.run_cycle(self.settings,retry_only=True)
            self.assertEqual(second['status'],'verified');self.assertEqual(create.call_count,1);self.assertEqual(send.call_count,2)
            self.assertEqual(self.state()['phase'],'complete');self.assertNotIn('next_retry_at_ts',self.state())
    def test_busy_deferrals_do_not_consume_error_budget_or_call_remote(self):
        with patch.object(schedule,'create_local',side_effect=RuntimeError('checkpoint_foreground_not_idle')),patch.object(schedule,'send_remote') as send:
            for _ in range(5):
                result=schedule.run_cycle(self.settings,retry_only=self.state_path_exists())
                self.assertEqual(result['status'],'retry_scheduled');self.assertEqual(self.state()['attempt'],0);self.due()
            send.assert_not_called()
    def state_path_exists(self):return (self.root/'data/runtime-backup-schedule.json').exists()
    def test_bounded_backoff_and_exhaustion_are_visible_and_preserve_checkpoint(self):
        with patch.object(schedule,'create_local',side_effect=self.create) as create,patch.object(schedule,'send_remote',side_effect=RuntimeError('vault_remote_receive_failed')):
            first=schedule.run_cycle(self.settings);checkpoint=self.state()['checkpoint']
            self.assertEqual(self.state()['next_retry_at_ts']-self.now,900)
            self.due();schedule.run_cycle(self.settings,retry_only=True)
            self.assertEqual(self.state()['next_retry_at_ts']-self.now,1800)
            self.due();last=schedule.run_cycle(self.settings,retry_only=True)
            self.assertEqual(last['status'],'attention_required');self.assertEqual(self.state()['notice_key'],'backup.attention_required')
            self.assertTrue(Path(checkpoint).is_dir());self.assertEqual(create.call_count,1)
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'not_due')
    def test_permanent_crypto_failure_never_falls_back_to_plaintext_or_retries_blindly(self):
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',side_effect=RuntimeError('vault_key_not_private')) as send:
            result=schedule.run_cycle(self.settings);self.assertEqual(result['status'],'attention_required')
            self.assertEqual(self.state()['attempt'],1);self.assertNotIn('next_retry_at_ts',self.state());self.assertEqual(send.call_count,1)
    def test_interruption_after_local_publication_reuses_the_same_snapshot(self):
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',side_effect=RuntimeError('vault_remote_receive_failed')):schedule.run_cycle(self.settings)
        state=self.state();state.update(phase='local',resume_phase='local');state.pop('next_retry_at_ts',None)
        write_json(self.root/'data/runtime-backup-schedule.json',state)
        with patch.object(schedule,'create_local') as create,patch.object(schedule,'send_remote',return_value={'status':'verified_encrypted'}):
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'verified');create.assert_not_called()
    def test_initial_retry_tick_does_not_create_invalid_state_or_start_backup(self):
        with patch.object(schedule,'create_local') as create:
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'not_due');create.assert_not_called()
        self.assertEqual(self.state()['schema_version'],1);self.assertEqual(self.state()['phase'],'idle')
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',return_value={'status':'verified'}):
            self.assertEqual(schedule.run_cycle(self.settings)['status'],'verified')
    def test_exception_details_and_credentials_are_not_persisted_in_notices(self):
        with patch.object(schedule,'create_local',side_effect=ValueError('PRIVATE_CREDENTIAL_SENTINEL')):
            schedule.run_cycle(self.settings)
        self.assertNotIn('PRIVATE_CREDENTIAL_SENTINEL',json.dumps(self.state()))
        self.assertEqual(self.state()['error_code'],'backup_operation_failed')
    def test_deadline_stops_old_cycle_without_deleting_its_local_backup(self):
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',side_effect=RuntimeError('vault_remote_receive_failed')):schedule.run_cycle(self.settings)
        path=Path(self.state()['checkpoint']);self.now+=86401
        self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'attention_required');self.assertTrue(path.exists())


    def test_capacity_deferrals_keep_existing_backups_without_consuming_retry_budget(self):
        blocked={'schema_version':1,'status':'insufficient','destination_shortfall_bytes':100,'source_shortfall_bytes':0}
        with patch.object(schedule,'capacity_report',return_value=blocked),patch.object(schedule,'create_local') as create,patch.object(schedule,'send_remote') as send:
            for _ in range(5):
                result=schedule.run_cycle(self.settings,retry_only=self.state_path_exists())
                self.assertEqual(result['phase'],'deferred');self.assertEqual(self.state()['attempt'],0)
                self.assertEqual(self.state()['notice_key'],'backup.capacity_deferred');self.due()
            create.assert_not_called();send.assert_not_called()
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',return_value={'status':'verified'}):
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'verified')

    def test_retry_resumes_after_deployed_cohort_is_recognized(self):
        with patch.object(schedule.checkpoint, 'running_revision', side_effect=RuntimeError('checkpoint_cohort_mismatch')), patch.object(schedule, 'create_local', side_effect=RuntimeError('checkpoint_cohort_mismatch')):
            first = schedule.run_cycle(self.settings)
        self.assertEqual(first['status'], 'attention_required')
        with patch.object(schedule, 'create_local', side_effect=self.create), patch.object(schedule, 'send_remote', return_value={'status': 'verified'}):
            second = schedule.run_cycle(self.settings, retry_only=True)
        self.assertEqual(second['status'], 'verified')

    def test_new_running_cohort_requests_one_fresh_backup_after_success(self):
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',return_value={'status':'verified'}):
            self.assertEqual(schedule.run_cycle(self.settings)['status'],'verified')
        self.now+=20
        def replacement(settings):
            result=self.create(settings);manifest=Path(result['checkpoint'])/'manifest.json'
            value=json.loads(manifest.read_text());value['source_revision']='b'*40;write_json(manifest,value);return result
        with patch.object(schedule.checkpoint,'running_revision',return_value='b'*40),patch.object(schedule,'create_local',side_effect=replacement) as create,patch.object(schedule,'send_remote',return_value={'status':'verified'}):
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'verified')
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'not_due')
            self.assertEqual(create.call_count,1)

    def test_observational_version_probe_failure_does_not_restart_a_complete_cycle(self):
        with patch.object(schedule,'create_local',side_effect=self.create),patch.object(schedule,'send_remote',return_value={'status':'verified'}):schedule.run_cycle(self.settings)
        with patch.object(schedule.checkpoint,'running_revision',side_effect=RuntimeError('fixture daemon offline')),patch.object(schedule,'create_local') as create:
            self.assertEqual(schedule.run_cycle(self.settings,retry_only=True)['status'],'not_due');create.assert_not_called()

    def test_capacity_race_at_snapshot_admission_does_not_consume_failure_budget(self):
        with patch.object(schedule,'capacity_report',return_value={'status':'ready'}),patch.object(schedule,'create_local',side_effect=RuntimeError('checkpoint_destination_low_space')),patch.object(schedule,'send_remote') as send:
            result=schedule.run_cycle(self.settings)
            self.assertEqual(result['attempt'],0)
            self.assertEqual(result['phase'],'deferred')
            self.assertEqual(self.state()['notice_key'],'backup.capacity_deferred')
            send.assert_not_called()


if __name__=='__main__':unittest.main()
