import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from run_privileged_secure_fs_case import TEST_NAME, SDK_TEST_NAME, SDK_SUCCESS_MARKER, resolve_executable, run_case


class PrivilegedFilesystemCaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='agent-secure-fs-runner-')
        self.addCleanup(self.directory.cleanup)
        # macOS temp paths can start with /var while their canonical location
        # is /private/var. Compare filesystem identities, not spelling aliases.
        self.workspace = Path(self.directory.name).resolve(strict=True)
        self.executable = self.workspace / 'target/debug/deps/fixture-clawd'
        self.executable.parent.mkdir(parents=True)
        self.executable.write_bytes(b'fixture executable, never run')
        self.executable.chmod(0o700)
        self.item = {'reason':'compiler-artifact','target':{'name':'clawd','kind':['bin']},
                     'profile':{'test':True},'executable':str(self.executable)}

    def resolve(self, item=None):
        return resolve_executable(json.dumps(self.item if item is None else item), self.workspace)

    def test_exact_cargo_test_binary_required(self):
        self.assertEqual(self.resolve(),self.executable)
        self.assertEqual(resolve_executable(json.dumps(self.item)+'\n'+json.dumps(self.item), self.workspace),self.executable)

    def test_parent_directory_alias_returns_the_canonical_executable(self):
        alias = self.workspace / 'workspace-alias'
        alias.symlink_to(self.workspace, target_is_directory=True)
        item = copy.deepcopy(self.item)
        item['executable'] = str(alias / self.executable.relative_to(self.workspace))
        self.assertEqual(resolve_executable(json.dumps(item), alias), self.executable)

    def test_parent_directory_alias_cannot_escape_target(self):
        outside = self.workspace / 'outside'
        outside.mkdir()
        executable = outside / 'fixture-clawd'
        executable.write_bytes(b'fixture executable, never run')
        executable.chmod(0o700)
        alias = self.workspace / 'target/outside-alias'
        alias.symlink_to(outside, target_is_directory=True)
        item = copy.deepcopy(self.item)
        item['executable'] = str(alias / executable.name)
        with self.assertRaises(ValueError):
            self.resolve(item)

    def test_wrong_target_profile_and_missing_executable_rejected(self):
        for path,value in [('name','other'),('kind',['lib']),('test',False),('test','true'),('executable',None)]:
            item=copy.deepcopy(self.item)
            if path in ['name','kind']: item['target'][path]=value
            elif path=='test': item['profile'][path]=value
            else: item[path]=value
            with self.assertRaises(ValueError): self.resolve(item)

    def test_outside_nonexecutable_and_symlink_rejected(self):
        outside=self.workspace/'outside'
        outside.write_bytes(b'fixture')
        outside.chmod(0o700)
        self.item['executable']=str(outside)
        with self.assertRaises(ValueError): self.resolve()
        self.item['executable']=str(self.executable)
        self.executable.chmod(0o600)
        with self.assertRaises(ValueError): self.resolve()
        self.executable.unlink()
        self.executable.symlink_to(outside)
        with self.assertRaises(ValueError): self.resolve()

    def test_ambiguous_or_malformed_artifacts_rejected(self):
        other=copy.deepcopy(self.item)
        other['executable']=str(self.workspace/'target/debug/deps/other')
        with self.assertRaises(ValueError):
            resolve_executable(json.dumps(self.item)+'\n'+json.dumps(other),self.workspace)
        with self.assertRaises(ValueError): resolve_executable('invalid JSON',self.workspace)

    def test_only_the_exact_fixture_is_invoked_without_a_shell(self):
        log='secure_workspace_metadata_preserved=true privileged_owner_check=true\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n'
        calls=[]
        def run(args,**kwargs):
            calls.append((args,kwargs))
            return subprocess.CompletedProcess(args,0,log)
        with contextlib.redirect_stdout(io.StringIO()): run_case(self.executable,run)
        self.assertEqual(calls[0][0],['sudo','-n',str(self.executable),TEST_NAME,'--exact','--nocapture'])
        self.assertNotIn('shell',calls[0][1])

    def test_root_mode_and_a_complete_single_success_are_mandatory(self):
        for code,log in [(1,'test result: ok. 1 passed; 0 failed; 0 ignored;'),
                         (0,'secure_workspace_metadata_preserved=true privileged_owner_check=false\ntest result: ok. 1 passed; 0 failed; 0 ignored;'),
                         (0,'secure_workspace_metadata_preserved=true privileged_owner_check=true\ntest result: ok. 0 passed; 0 failed; 1 ignored;')]:
            with self.assertRaises(ValueError), contextlib.redirect_stdout(io.StringIO()):
                run_case(self.executable,lambda args,**kwargs:subprocess.CompletedProcess(args,code,log))

    def test_sdk_mode_requires_the_exact_cargo_library_test_target(self):
        item = copy.deepcopy(self.item)
        item['target'] = {'name': 'skill_sdk', 'kind': ['lib']}
        self.assertEqual(resolve_executable(json.dumps(item), self.workspace,
                         target_name='skill_sdk', target_kind='lib'), self.executable)
        for target in [{'name': 'skill_sdk', 'kind': ['bin']}, {'name': 'other', 'kind': ['lib']}]:
            bad = copy.deepcopy(item)
            bad['target'] = target
            with self.assertRaises(ValueError):
                resolve_executable(json.dumps(bad), self.workspace, target_name='skill_sdk', target_kind='lib')

    def test_sdk_mode_invokes_only_its_exact_native_fixture(self):
        calls = []
        log = SDK_SUCCESS_MARKER + '\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n'
        def run(args, **kwargs):
            calls.append(args)
            return subprocess.CompletedProcess(args, 0, log)
        with contextlib.redirect_stdout(io.StringIO()):
            run_case(self.executable, run, test_name=SDK_TEST_NAME, success_marker=SDK_SUCCESS_MARKER)
        self.assertEqual(calls, [['sudo', '-n', str(self.executable), SDK_TEST_NAME, '--exact', '--nocapture']])

    def test_sdk_mode_rejects_non_root_or_missing_unpreservable_owner_proof(self):
        for marker in [SDK_SUCCESS_MARKER.replace('privileged_owner_check=true', 'privileged_owner_check=false'),
                       SDK_SUCCESS_MARKER.replace('unpreservable_owner_rejected=true', 'unpreservable_owner_rejected=false')]:
            log = marker + '\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n'
            with self.assertRaises(ValueError), contextlib.redirect_stdout(io.StringIO()):
                run_case(self.executable, lambda args, **kwargs: subprocess.CompletedProcess(args, 0, log),
                         test_name=SDK_TEST_NAME, success_marker=SDK_SUCCESS_MARKER)


if __name__ == '__main__': unittest.main()
