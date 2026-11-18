"""Keep native Mac verification on supported runners without changing target OS floors."""
from copy import deepcopy
from pathlib import Path
import re
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
EXPECTED = (
    ("macos-polyglot-skill-ci.yml", "polyglot-skill-macos", "macos-15", None),
    ("macos-arm64-release-artifact.yml", "build-macos",
     "${{ inputs.architecture == 'x86_64' && 'macos-15-intel' || 'macos-15' }}", "13.0"),
    ("release-python-wheels.yml", "macos-wheels",
     "${{ startsWith(inputs.target, 'aarch64') && 'macos-15' || 'macos-15-intel' }}", "13.0"),
    ("desktop-native.yml", "desktop",
     "${{ matrix.platform == 'windows-x64' && 'windows-2022' || matrix.platform == 'macos-arm64' && 'macos-15' || 'macos-15-intel' }}", "13.0"),
)


def load(name):
    return yaml.safe_load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"))


def valid_job(job, runner, target):
    return job.get("runs-on") == runner and (
        target is None or str(job.get("env", {}).get("MACOSX_DEPLOYMENT_TARGET")) == target
    )


def required_reload_gate(job, selector="skill_reload_tests"):
    command = f"cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture"
    return required_command_gate(job, command)


def required_command_gate(job, command):
    selected = [step for step in job["steps"] if command in step.get("run", "")]
    return len(selected) == 1 and (
        "if" not in selected[0] and not selected[0].get("continue-on-error", False)
        and "set -euo pipefail" in selected[0]["run"]
        and sum(line.strip() == command for line in selected[0]["run"].splitlines()) == 1
    )


def required_builtin_memory_gate(job):
    prefix='skills::builtin::builtin_memory_store::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    self_test='python3 scripts/tests/assert_rust_test_selection.py --self-test'
    required=['save_is_effectively_once_and_search_returns_only_opaque_identity',
        'conversation_scope_isolated_and_child_mutation_is_denied',
        'pagination_scope_revision_forget_and_disabled_generation_are_enforced',
        'builtin_saved_fact_can_be_deleted_with_its_revision_and_index',
        'consistency::healthy_fact_is_not_reported_or_deleted_as_orphan',
        'consistency::repair_removes_only_the_actual_orphan_and_preserves_other_owners']
    return listing in script and self_test in script and '|| true' not in script and (
        script.index(self_test)<script.index(listing)<script.index(run)
        and f'--prefix {prefix}tests::' in script
        and all(f'--require {name}' in script for name in required))


def required_feedback_memory_gate(job):
    prefix='memory::ux::feedback_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    expected=set()
    for module,file in [('failures','ux_feedback_failure_tests.rs'),('success','ux_feedback_success_tests.rs')]:
        source=(ROOT/'crates/clawd/src/memory'/file).read_text(encoding='utf-8')
        expected.update(module+'::'+name for name in re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return bool(expected) and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_recall_memory_gate(job):
    prefix='memory::ux::recall_contract_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    expected=set()
    for module,file in [('corrections','ux_recall_correction_tests.rs'),('disabling','ux_recall_disable_tests.rs'),
        ('incremental','ux_recall_incremental_tests.rs'),('boundaries','ux_recall_boundary_tests.rs'),
        ('derived','ux_recall_derived_tests.rs')]:
        source=(ROOT/'crates/clawd/src/memory'/file).read_text(encoding='utf-8')
        expected.update(module+'::'+name for name in re.findall(r'^#\[(?:tokio::)?test\]\n(?:async )?fn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==30 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_projection_transaction_gate(job):
    prefix='memory::projection_transaction::tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    source=(ROOT/'crates/clawd/src/memory/projection_transaction_tests.rs').read_text(encoding='utf-8')
    expected=set(re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==4 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_inflight_embedding_gate(job):
    prefix='memory::embedding_jobs::capacity_http_tests::recall_races::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    source=(ROOT/'crates/clawd/src/memory/embedding_recall_race_tests.rs').read_text(encoding='utf-8')
    expected=set(re.findall(r'^#\[tokio::test\]\nasync fn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==8 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_embedding_commit_gate(job):
    prefix='memory::embedding_jobs::capacity_http_tests::response_commit::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    source=(ROOT/'crates/clawd/src/memory/embedding_commit_tests.rs').read_text(encoding='utf-8')
    expected=set(re.findall(r'^#\[tokio::test\]\nasync fn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==8 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_snapshot_maintenance_gate(job):
    prefix='memory::embedding_jobs::capacity_http_tests::snapshot_maintenance::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    source=(ROOT/'crates/clawd/src/memory/embedding_snapshot_tests.rs').read_text(encoding='utf-8')
    expected=set(re.findall(r'^#\[tokio::test\]\nasync fn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==4 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_memory_actor_gate(job):
    prefix='http::memory_routes::actor_fence_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    source=(ROOT/'crates/clawd/src/http/memory_actor_fence_tests.rs').read_text(encoding='utf-8')
    expected=set(re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==14 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_remaining_memory_writer_gate(job):
    prefix='http::memory_routes::write_fence_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    source=(ROOT/'crates/clawd/src/http/memory_write_fence_tests.rs').read_text(encoding='utf-8')
    expected=set(re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M))
    required=set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))
    return len(expected)==33 and required==expected and listing in script and (
        'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
        and f'--prefix {prefix}' in script and '|| true' not in script
        and script.index(listing)<script.index(run))


def required_retained_memory_vector_gate(job):
    for prefix,count,file in [
        ('http::memory_routes::vector_policy_tests::',3,'memory_vector_policy_tests.rs'),
        ('http::memory_routes::tests::',1,'memory_routes_tests.rs'),
    ]:
        run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
        if not required_command_gate(job,run):
            return False
        step=next(step for step in job['steps'] if run in step.get('run',''))
        script=step['run']
        listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
        source=(ROOT/'crates/clawd/src/http'/file).read_text(encoding='utf-8')
        names=set(re.findall(r'^#\[(?:tokio::)?test\]\n(?:async )?fn (\w+)\(',source,re.M))
        if not (len(names)==count and listing in script and f'--prefix {prefix}' in script
                and all(f'--require {name}' in script for name in names)
                and '|| true' not in script and script.index(listing)<script.index(run)):
            return False
    return True


def required_reindex_startup_gate(job):
    pairs=[
        ('memory::embedding_jobs::tests::remote_reindex_requires_explicit_consent_before_outbound_jobs_exist',
         'memory::embedding_jobs::tests::','remote_reindex_requires_explicit_consent_before_outbound_jobs_exist'),
        ('memory::embedding_jobs::rejected_startup_tests::',
         'memory::embedding_jobs::rejected_startup_tests::','denied_reindex_does_not_install_schema_receipts_or_profile_state'),
    ]
    for selector,prefix,name in pairs:
        run=f'cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture'
        if not required_command_gate(job,run):
            return False
        step=next(step for step in job['steps'] if run in step.get('run',''))
        script=step['run']
        listing=f'cargo test --locked -p clawd --bin clawd {selector} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
        if not (listing in script and f'--prefix {prefix}' in script and f'--require {name}' in script
                and '|| true' not in script and script.index(listing)<script.index(run)):
            return False
    return True


def required_domain_budget_gate(job):
    prefix='memory::ux::budget_contract_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    names=set()
    for module,file,count in [('growth','growth',5),('controls','control',8)]:
        source=(ROOT/'crates/clawd/src/memory'/f'ux_budget_{file}_tests.rs').read_text(encoding='utf-8')
        cases=re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M)
        if len(cases)!=count or len(set(cases))!=count:
            return False
        names.update(module+'::'+name for name in cases)
    full=next(step for step in job['steps'] if step.get('name')=='Test memory quota byte accounting and empty-principal refresh')
    return (listing in script and f'--prefix {prefix}' in script
            and set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))==names
            and '|| true' not in script and script.index(listing)<script.index(run)
            and job['steps'].index(step)<job['steps'].index(full))


def required_growth_state_gate(job):
    pairs=[
        ('memory::ux::growth_state_tests::','memory::ux::growth_state_tests::'),
        ('memory::ux::admission_contract_tests::missing_pressure_correction_fails_closed_without_repairing_the_observation',
         'memory::ux::admission_contract_tests::'),
    ]
    for selector,prefix in pairs:
        run=f'cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture'
        if not required_command_gate(job,run):
            return False
        step=next(step for step in job['steps'] if run in step.get('run',''))
        script=step['run']
        listing=f'cargo test --locked -p clawd --bin clawd {selector} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
        if selector.endswith('growth_state_tests::'):
            source=(ROOT/'crates/clawd/src/memory/ux_growth_state_tests.rs').read_text(encoding='utf-8')
            names=re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M)
            if len(names)!=8 or len(set(names))!=8:
                return False
        else:
            names=['missing_pressure_correction_fails_closed_without_repairing_the_observation']
        budget=next(value for value in job['steps'] if value.get('name')=='Test actual memory domain budget and effect contracts')
        if not (listing in script and f'--prefix {prefix}' in script and '|| true' not in script
                and all(f'--require {name}' in script for name in names)
                and script.index(listing)<script.index(run)
                and job['steps'].index(step)<job['steps'].index(budget)):
            return False
    return True


def required_budget_policy_gate(job):
    prefix='memory::ux::budget_policy_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    source=(ROOT/'crates/clawd/src/memory/ux_budget_policy_tests.rs').read_text(encoding='utf-8')
    names=re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M)
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    prior=next(value for value in job['steps'] if value.get('name')=='Test invalid capacity state admission and recovery controls')
    script=step['run']
    return (len(names)==12 and len(set(names))==12 and listing in script
            and f'--prefix {prefix}' in script and '|| true' not in script
            and set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))==set(names)
            and script.index(listing)<script.index(run)
            and job['steps'].index(step)<job['steps'].index(prior))


def required_manual_growth_observation_gate(job):
    pairs=[
        ('memory::ux::growth_availability_tests::','memory/ux_growth_availability_tests.rs',8),
        ('http::memory_routes::capacity::growth_tests::','http/memory_growth_capacity_tests.rs',4),
    ]
    for prefix,path,count in pairs:
        run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
        if not required_command_gate(job,run):
            return False
        step=next(step for step in job['steps'] if run in step.get('run',''))
        source=(ROOT/'crates/clawd/src'/path).read_text(encoding='utf-8')
        names=re.findall(r'^#\[(?:tokio::)?test\]\n(?:async )?fn (\w+)\(',source,re.M)
        listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
        prior=next(value for value in job['steps'] if value.get('name')=='Test configured memory budget and source-only quota contracts')
        script=step['run']
        if not (len(names)==count and len(set(names))==count and listing in script
                and f'--prefix {prefix}' in script and '|| true' not in script
                and all(f'--require {name}' in script for name in names)
                and script.index(listing)<script.index(run)
                and job['steps'].index(step)<job['steps'].index(prior)):
            return False
    return True


def required_lazy_domain_sampling_gate(job):
    prefix='memory::availability::sampling_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    source=(ROOT/'crates/clawd/src/memory/availability_sampling_tests.rs').read_text(encoding='utf-8')
    names=re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M)
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    prior=next(value for value in job['steps'] if value.get('name')=='Test manual memory growth availability and read-only HTTP projection')
    script=step['run']
    return (len(names)==10 and len(set(names))==10 and listing in script
            and f'--prefix {prefix}' in script and '|| true' not in script
            and set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))==set(names)
            and script.index(listing)<script.index(run)
            and job['steps'].index(step)<job['steps'].index(prior))


def required_capacity_read_authentication_gate(job):
    prefix='http::memory_capacity_read_race_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    source=(ROOT/'crates/clawd/src/http/memory_capacity_read_race_tests.rs').read_text(encoding='utf-8')
    names=re.findall(r'^#\[test\]\nfn (\w+)\(',source,re.M)
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    prior=next(value for value in job['steps'] if value.get('name')=='Test memory vector status policy and scoped controls')
    script=step['run']
    return (len(names)==6 and len(set(names))==6 and listing in script
            and f'--prefix {prefix}' in script and '|| true' not in script
            and set(re.findall(r'--require ([A-Za-z0-9_:]+)',script))==set(names)
            and script.index(listing)<script.index(run)
            and job['steps'].index(step)==job['steps'].index(prior)+1)


def required_memory_read_boundary_gate(job):
    prefix='http::memory_read_boundary_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    source=(ROOT/'crates/clawd/src/http/memory_read_boundary_tests.rs').read_text(encoding='utf-8')
    routes=re.findall(r'read_contract!\((\w+), "[^"]+"\);',source)
    cases=re.findall(r'#\[test\]\s+fn (\w+)\(',source)
    names={f'{route}::{case}' for route in routes for case in cases}
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    prior=next(value for value in job['steps'] if value.get('name')=='Test capacity read authentication snapshot contracts')
    script=step['run']
    required=re.findall(r'--require ([A-Za-z0-9_:]+)',script)
    return (len(routes)==11 and len(cases)==6 and len(names)==66
            and len(required)==66 and set(required)==names and listing in script
            and f'--prefix {prefix}' in script and '|| true' not in script
            and script.index(listing)<script.index(run)
            and job['steps'].index(step)==job['steps'].index(prior)+1)


def required_project_read_gate(job):
    prefix='http::memory_project_read_tests::'
    run=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,run):
        return False
    step=next(step for step in job['steps'] if run in step.get('run',''))
    root=ROOT/'crates/clawd/src/http'
    names=re.findall(r'#\[test\]\s+fn (\w+)\(', (root/'memory_project_read_tests.rs').read_text())
    lookup=re.findall(r'#\[test\]\s+fn (\w+)\(', (root/'memory_project_lookup_tests.rs').read_text())
    names=names+['lookup::'+name for name in lookup]
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    prior=next(value for value in job['steps'] if value.get('name')=='Test memory read boundary authentication contracts')
    script=step['run']
    required=re.findall(r'--require ([A-Za-z0-9_:]+)',script)
    return (len(names)==12 and len(set(names))==12 and len(lookup)==6
            and len(required)==12 and set(required)==set(names) and listing in script
            and f'--prefix {prefix}' in script and '|| true' not in script
            and script.index(listing)<script.index(run)
            and job['steps'].index(step)==job['steps'].index(prior)+1)


def required_import_upload_gate(job):
    prefix='memory::import_upload::storage_tests::'
    command=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,command):
        return False
    step=next(step for step in job['steps'] if command in step.get('run',''))
    root=ROOT/'crates/clawd/src/memory/import_upload'
    expected=set()
    for module in ['schema','chunks','identity','leases','reader','accounting']:
        source=(root/(module+'_tests.rs')).read_text()
        expected.update(module+'::'+name for name in re.findall(r'#\[test\]\s+fn (\w+)\(',source))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    required=re.findall(r'--require ([A-Za-z0-9_:]+)',script)
    prior=next(value for value in job['steps'] if value.get('name')=='Test project lookup isolation and read-only identity contracts')
    return (len(expected)==26 and len(required)==26 and set(required)==expected
            and listing in script and f'--prefix {prefix}' in script and '|| true' not in script
            and "python3 -B -m unittest discover -s scripts/tests -p 'test_memory_import_upload_schema.py'" in script
            and script.index(listing)<script.index(command)
            and job['steps'].index(step)==job['steps'].index(prior)+1)


def required_streaming_import_gate(job):
    prefix='memory::import_upload::parser::tests::'
    command=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,command):
        return False
    step=next(step for step in job['steps'] if command in step.get('run',''))
    source=(ROOT/'crates/clawd/src/memory/import_upload/parser/tests.rs').read_text()
    expected=set(re.findall(r'#\[test\]\s+fn (\w+)\(',source))
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    required=re.findall(r'--require ([A-Za-z0-9_:]+)',script)
    prior=next(value for value in job['steps'] if value.get('name')=='Test durable memory import upload foundations')
    return (len(expected)==13 and len(required)==13 and set(required)==expected
            and listing in script and f'--prefix {prefix}' in script and '|| true' not in script
            and script.index(listing)<script.index(command)
            and job['steps'].index(step)==job['steps'].index(prior)+1)


def required_staged_import_integration_gate(job,kind):
    if kind=='shared':
        prefix='memory::ux::import_items::tests::'
        sources=[(None,ROOT/'crates/clawd/src/memory/ux_import_items_tests.rs')]
        count=9
        prior_name='Test bounded streaming memory export decoding'
    else:
        prefix='memory::import_upload::flow_tests::'
        sources=[(module,ROOT/('crates/clawd/src/memory/import_upload/flow_'+module+'_tests.rs'))
                 for module in ['roundtrip','failure','lifecycle','batching']]
        count=20
        prior_name='Test shared memory import filtering and canonical insertion'
    command=f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job,command):
        return False
    step=next(step for step in job['steps'] if command in step.get('run',''))
    expected=set()
    for module,path in sources:
        for name in re.findall(r'#\[test\]\s+fn (\w+)\(',path.read_text()):
            expected.add((module+'::' if module else '')+name)
    script=step['run']
    listing=f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    required=re.findall(r'--require ([A-Za-z0-9_:]+)',script)
    prior=next(value for value in job['steps'] if value.get('name')==prior_name)
    return (len(expected)==count and len(required)==count and set(required)==expected
            and listing in script and f'--prefix {prefix}' in script and '|| true' not in script
            and script.index(listing)<script.index(command)
            and job['steps'].index(step)==job['steps'].index(prior)+1)


def required_staged_http_gate(job,kind):
    if kind=='clawd':
        groups=[
            ('clawd','memory::import_upload::ingress::tests::',6,
             [(None,ROOT/'crates/clawd/src/memory/import_upload/ingress_tests.rs')]),
            ('clawd','http::memory_import_http_tests::',15,
             [(module,ROOT/('crates/clawd/src/http/memory_import_'+module+'_tests.rs'))
              for module in ['ingress','roundtrip','boundary']])]
        prior='Test staged memory whole preview confirmation and privacy lifecycle'
    else:
        groups=[('webd','request_limits::memory_import_tests::',4,
                 [(None,ROOT/'crates/webd/src/memory_import_limits_tests.rs')])]
        prior='Test bounded staged import HTTP and ingress policies'
    gate_step=None
    for package,prefix,count,sources in groups:
        run=f'cargo test --locked -p {package} --bin {package} {prefix} -- --test-threads=1 --nocapture'
        if not required_command_gate(job,run):
            return False
        step=next(step for step in job['steps'] if run in step.get('run',''))
        if gate_step is not None and step is not gate_step:
            return False
        gate_step=step
        expected=set()
        for module,path in sources:
            for name in re.findall(r'#\[(?:tokio::)?test\]\s+(?:async\s+)?fn (\w+)\(',path.read_text()):
                expected.add((module+'::' if module else '')+name)
        listing=f'cargo test --locked -p {package} --bin {package} {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
        script=step['run']
        if listing not in script or script.index(listing)>script.index(run):
            return False
        segment=script[script.index(listing):script.index(run)]
        required=re.findall(r'--require ([A-Za-z0-9_:]+)',segment)
        if not(len(expected)==count and len(required)==count and set(required)==expected
               and f'--prefix {prefix}' in segment and '|| true' not in script):
            return False
    previous=next(step for step in job['steps'] if step.get('name')==prior)
    return (job['steps'].index(gate_step)==job['steps'].index(previous)+1
            and (kind!='webd' or 'cargo test --locked -p webd --bin webd -- --test-threads=1 --nocapture' in gate_step['run']))


def required_signed_webd_bundle(job):
    names = [
        'Test native webd staged import transport limits',
        'Build the exact verified Linux core without publishing a release',
        'Attest the exact Linux core provenance',
        'Preserve the exact verified Linux core artifact',
    ]
    steps = job['steps']
    if any(sum(step.get('name') == name for step in steps) != 1 for name in names):
        return False
    gate, build, attest, upload = [next(step for step in steps if step.get('name') == name)
                                   for name in names]
    if not (required_staged_http_gate(job, 'clawd')
            and required_staged_http_gate(job, 'webd')
            and steps.index(gate) < steps.index(build) < steps.index(attest) < steps.index(upload)
            and all('if' not in step for step in [build, upload])
            and all(not step.get('continue-on-error', False) for step in [build, attest, upload])):
        return False
    lines = [line.strip() for line in build.get('run', '').splitlines() if line.strip()]
    gate_lines = [line for line in lines if re.fullmatch(
        r'''printf '%s\\n' '[a-z0-9_ ]+' > "\$artifact_root/runtime-read/verified-gates"''', line)]
    if len(gate_lines) != 1:
        return False
    expected = [
        'set -euo pipefail',
        'cargo build --locked --release -p clawd --bin clawd -p clawcli --bin clawcli -p webd --bin webd',
        'artifact_root="$RUNNER_TEMP/runtime-read-artifact"',
        'mkdir -p "$artifact_root/runtime-read"',
        'install -m 755 target/release/clawd "$artifact_root/runtime-read/clawd"',
        'install -m 755 target/release/clawcli "$artifact_root/runtime-read/clawcli"',
        'install -m 755 target/release/webd "$artifact_root/runtime-read/webd"',
        'printf \'%s\\n\' "$GITHUB_SHA" > "$artifact_root/runtime-read/source-commit"',
        gate_lines[0],
        'printf \'%s\\n\' \'memory_staged_import_http_native webd_import_transport_native\' >> "$artifact_root/runtime-read/verified-gates"',
        '(cd "$artifact_root/runtime-read" && sha256sum clawd clawcli source-commit verified-gates > SHA256SUMS)',
        '(cd "$artifact_root/runtime-read" && sha256sum webd >> SHA256SUMS)',
        'tar -czf "$artifact_root/runtime-read-${GITHUB_SHA}.tar.gz" -C "$artifact_root" runtime-read',
        '(cd "$artifact_root" && sha256sum "runtime-read-${GITHUB_SHA}.tar.gz" > "runtime-read-${GITHUB_SHA}.tar.gz.sha256")',
    ]
    return (lines == expected
            and attest.get('if') == "${{ github.event_name != 'pull_request' }}"
            and attest.get('uses') == 'actions/attest-build-provenance@43d14bc2b83dec42d39ecae14e916627a18bb661'
            and attest.get('with') == {
                'subject-path': '${{ runner.temp }}/runtime-read-artifact/runtime-read-${{ github.sha }}.tar.gz'}
            and upload.get('uses') == 'actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02'
            and upload.get('with') == {
                'name': 'verified-linux-runtime-read-${{ github.sha }}',
                'path': '${{ runner.temp }}/runtime-read-artifact/*.tar.gz*',
                'if-no-files-found': 'error', 'retention-days': 7}
            and job.get('permissions', {}).get('id-token') == 'write'
            and job.get('permissions', {}).get('attestations') == 'write')


def required_memory_configuration_gate(job):
    prefix = 'bootstrap::config_loaders::'
    run = f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job, run):
        return False
    step = next(step for step in job['steps'] if run in step.get('run', ''))
    root = ROOT / 'crates/clawd/src/bootstrap'
    source = (root / 'memory_runtime_config_tests.rs').read_text()
    names = re.findall(r'^#\[test\]\nfn (\w+)\(', source, re.M)
    expected = {'memory_runtime_config_tests::' + name for name in names}
    expected.add('tests::memory_runtime_file_controls_startup_reindex_and_background_concurrency')
    script = step['run']
    listing = f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    if listing not in script or script.index(listing) > script.index(run):
        return False
    required = re.findall(r'--require ([A-Za-z0-9_:]+)', script)
    previous = next(step for step in job['steps'] if step.get('name') == 'Test native webd staged import transport limits')
    loader = (root / 'config_loaders.rs').read_text()
    return (len(names) == len(set(names)) == 7 and len(required) == 8 and set(required) == expected
            and f'--prefix {prefix}' in script and '|| true' not in script
            and 'python3 scripts/tests/assert_rust_test_selection.py --self-test' in script
            and job['steps'].index(step) == job['steps'].index(previous) + 1
            and '#[path = "memory_runtime_config_tests.rs"]\nmod memory_runtime_config_tests;' in loader)


def required_owned_retirement_gate(job):
    prefix = 'memory::retirement_access::'
    run = f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
    if not required_command_gate(job, run):
        return False
    step = next(step for step in job['steps'] if run in step.get('run', ''))
    root = ROOT / 'crates/clawd/src/memory'
    owner = (root / 'retirement_access.rs').read_text()
    expected = set()
    for module, filename, count in [
        ('migration_tests', 'retirement_access_migration_tests.rs', 12),
        ('tests', 'retirement_access_tests.rs', 8),
        ('failure_tests', 'retirement_access_failure_tests.rs', 3),
        ('work_tests', 'retirement_access_work_tests.rs', 3),
    ]:
        source = (root / filename).read_text()
        names = re.findall(r'^#\[test\]\nfn (\w+)\(', source, re.M)
        if (len(names) != len(set(names)) or len(names) != count or '#[ignore' in source
                or f'#[cfg(test)]\n#[path = "{filename}"]\nmod {module};' not in owner):
            return False
        expected.update(module + '::' + name for name in names)
    script = step['run']
    listing = f'cargo test --locked -p clawd --bin clawd {prefix} -- --list --color never | python3 scripts/tests/assert_rust_test_selection.py'
    self_test = 'python3 scripts/tests/assert_rust_test_selection.py --self-test'
    previous = [item for item in job['steps'] if item.get('name') == 'Test effective flat and wrapped memory configuration']
    required = re.findall(r'--require ([A-Za-z0-9_:]+)', script)
    return (step.get('name') == 'Test owned memory retirement access paths'
            and step.get('shell') == 'bash' and len(previous) == 1
            and job['steps'].index(step) == job['steps'].index(previous[0]) + 1
            and script.splitlines().count(listing + ' \\') == 1
            and script.splitlines().count(self_test) == 1
            and script.index(self_test) < script.index(listing) < script.index(run)
            and re.findall(r'--prefix ([A-Za-z0-9_:]+)', script) == [prefix]
            and len(required) == len(expected) == 26 and set(required) == expected
            and '|| true' not in script and '--ignored' not in script)


class MacosRunnerWorkflowTests(unittest.TestCase):
    def test_owned_retirement_gates_cover_every_native_case_without_bypasses(self):
        jobs = load('macos-polyglot-skill-ci.yml')['jobs']
        prefix = 'memory::retirement_access::'
        run = f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
        for name in ['runtime-read-linux', 'polyglot-skill-macos']:
            self.assertTrue(required_owned_retirement_gate(jobs[name]))
            original = next(step for step in jobs[name]['steps'] if run in step.get('run', ''))
            cases = re.findall(r'--require ([A-Za-z0-9_:]+)', original['run'])
            mutations = ['remove', 'conditional', 'continue', 'ignore', 'duplicate-case',
                         'wrong-prefix', 'run-before-list', 'ignore-list', 'moved-step',
                         'without-self-test', 'without-pipefail'] + ['missing:' + case for case in cases]
            for mutation in mutations:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job['steps'] if run in step.get('run', ''))
                    script = step['run']
                    if mutation == 'remove':
                        job['steps'].remove(step)
                    elif mutation == 'conditional':
                        step['if'] = '${{ false }}'
                    elif mutation == 'continue':
                        step['continue-on-error'] = True
                    elif mutation == 'ignore':
                        step['run'] = script.replace(run, run + ' || true')
                    elif mutation == 'duplicate-case':
                        case = '--require ' + cases[0]
                        step['run'] = script.replace(case, case + ' ' + case)
                    elif mutation == 'wrong-prefix':
                        step['run'] = script.replace('--prefix ' + prefix, '--prefix missing::')
                    elif mutation == 'run-before-list':
                        step['run'] = script.replace(run + '\n', '')
                        step['run'] = step['run'].replace('set -euo pipefail\n', 'set -euo pipefail\n' + run + '\n')
                    elif mutation == 'ignore-list':
                        step['run'] = script.replace(' | python3 scripts/tests/assert_rust_test_selection.py', ' || true | python3 scripts/tests/assert_rust_test_selection.py')
                    elif mutation == 'moved-step':
                        at = job['steps'].index(step)
                        job['steps'].remove(step)
                        job['steps'].insert(at + 1, step)
                    elif mutation == 'without-self-test':
                        step['run'] = script.replace('python3 scripts/tests/assert_rust_test_selection.py --self-test\n', '')
                    elif mutation == 'without-pipefail':
                        step['run'] = script.replace('set -euo pipefail', 'set -e')
                    else:
                        step['run'] = script.replace('--require ' + mutation.split(':', 1)[1], '')
                    self.assertFalse(required_owned_retirement_gate(job))

    def test_memory_configuration_gates_require_both_shapes_and_the_existing_native_case(self):
        jobs = load('macos-polyglot-skill-ci.yml')['jobs']
        prefix = 'bootstrap::config_loaders::'
        run = f'cargo test --locked -p clawd --bin clawd {prefix} -- --test-threads=1 --nocapture'
        for name in ['runtime-read-linux', 'polyglot-skill-macos']:
            self.assertTrue(required_memory_configuration_gate(jobs[name]))
            for mutation in ['remove', 'conditional', 'continue', 'ignore', 'missing-case',
                             'missing-existing-case', 'duplicate-case', 'wrong-prefix',
                             'run-before-list', 'ignore-list', 'moved-step']:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job['steps'] if run in step.get('run', ''))
                    script = step['run']
                    case = '--require memory_runtime_config_tests::flat_explicit_false_and_nondefault_values_are_preserved'
                    if mutation == 'remove':
                        job['steps'].remove(step)
                    elif mutation == 'conditional':
                        step['if'] = '${{ false }}'
                    elif mutation == 'continue':
                        step['continue-on-error'] = True
                    elif mutation == 'ignore':
                        step['run'] = script.replace(run, run + ' || true')
                    elif mutation == 'missing-case':
                        step['run'] = script.replace(case, '')
                    elif mutation == 'missing-existing-case':
                        step['run'] = script.replace('--require tests::memory_runtime_file_controls_startup_reindex_and_background_concurrency', '')
                    elif mutation == 'duplicate-case':
                        step['run'] = script.replace(case, case + ' ' + case)
                    elif mutation == 'wrong-prefix':
                        step['run'] = script.replace('--prefix ' + prefix, '--prefix missing::')
                    elif mutation == 'run-before-list':
                        step['run'] = script.replace(run + '\n', '')
                        step['run'] = step['run'].replace('set -euo pipefail\n', 'set -euo pipefail\n' + run + '\n')
                    elif mutation == 'ignore-list':
                        step['run'] = script.replace(' | python3 scripts/tests/assert_rust_test_selection.py', ' || true | python3 scripts/tests/assert_rust_test_selection.py')
                    else:
                        at = job['steps'].index(step)
                        job['steps'].remove(step)
                        job['steps'].insert(at + 1, step)
                    self.assertFalse(required_memory_configuration_gate(job))

    def test_signed_core_bundle_contains_exact_tested_webd_and_no_bypass(self):
        original = load('macos-polyglot-skill-ci.yml')['jobs']['runtime-read-linux']
        self.assertTrue(required_signed_webd_bundle(original))
        build_name = 'Build the exact verified Linux core without publishing a release'
        attest_name = 'Attest the exact Linux core provenance'
        upload_name = 'Preserve the exact verified Linux core artifact'
        for mutation in [
            'build-target', 'debug-binary', 'missing-stage', 'missing-hash', 'missing-marker',
            'stage-after-tar', 'hash-after-tar', 'source-sha', 'ignored-command',
            'upload-before-attest', 'build-before-tests', 'wrong-subject', 'unsigned-condition',
            'conditional-build', 'conditional-upload', 'continue-build', 'continue-attest',
            'continue-upload', 'missing-artifact', 'unpinned-attest', 'id-token',
        ]:
            with self.subTest(mutation=mutation):
                job = deepcopy(original)
                steps = job['steps']
                named = {step.get('name'): step for step in steps}
                build, attest, upload = [named[name] for name in [build_name, attest_name, upload_name]]
                script = build['run']
                if mutation == 'build-target':
                    build['run'] = script.replace(' -p webd --bin webd', '')
                elif mutation == 'debug-binary':
                    build['run'] = script.replace('target/release/webd', 'target/debug/webd')
                elif mutation in ['missing-stage', 'missing-hash', 'missing-marker']:
                    fragment = {'missing-stage': 'install -m 755 target/release/webd',
                                'missing-hash': 'sha256sum webd',
                                'missing-marker': 'memory_staged_import_http_native'}[mutation]
                    build['run'] = '\n'.join(line for line in script.splitlines() if fragment not in line)
                elif mutation in ['stage-after-tar', 'hash-after-tar']:
                    fragment = 'install -m 755 target/release/webd' if mutation == 'stage-after-tar' else 'sha256sum webd'
                    lines = script.splitlines()
                    moved = next(line for line in lines if fragment in line)
                    lines.remove(moved)
                    lines.insert(next(i for i, line in enumerate(lines) if 'tar -czf' in line) + 1, moved)
                    build['run'] = '\n'.join(lines)
                elif mutation == 'source-sha':
                    build['run'] = script.replace('$GITHUB_SHA', 'unverified-source')
                elif mutation == 'ignored-command':
                    build['run'] = script.replace(' -p webd --bin webd', ' -p webd --bin webd || true')
                elif mutation == 'upload-before-attest':
                    steps.remove(upload)
                    steps.insert(steps.index(attest), upload)
                elif mutation == 'build-before-tests':
                    steps.remove(build)
                    steps.insert(steps.index(named['Test native webd staged import transport limits']), build)
                elif mutation == 'wrong-subject':
                    attest['with']['subject-path'] = 'old-unsigned-gateway.tar.gz'
                elif mutation == 'unsigned-condition':
                    attest['if'] = '${{ false }}'
                elif mutation.startswith('conditional-'):
                    {'conditional-build': build, 'conditional-upload': upload}[mutation]['if'] = '${{ false }}'
                elif mutation.startswith('continue-'):
                    {'continue-build': build, 'continue-attest': attest,
                     'continue-upload': upload}[mutation]['continue-on-error'] = True
                elif mutation == 'missing-artifact':
                    upload['with']['if-no-files-found'] = 'ignore'
                elif mutation == 'unpinned-attest':
                    attest['uses'] = 'actions/attest-build-provenance@main'
                else:
                    job['permissions']['id-token'] = 'read'
                self.assertFalse(required_signed_webd_bundle(job))

    def test_staged_http_adapter_children_have_explicit_resolvable_paths(self):
        root=ROOT/'crates/clawd/src/http'
        source=(root/'memory_import.rs').read_text()
        for name in ['body','errors','execution']:
            expected=f'memory_import/{name}.rs'
            declarations=re.findall(r'#\[path = "([^"]+)"\]\s+mod '+name+r';',source)
            self.assertEqual(declarations,[expected])
            self.assertTrue((root/expected).is_file())

    def test_staged_http_and_webd_gates_keep_all_raw_body_and_policy_cases(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            for kind,label in [
                ('clawd','Test bounded staged import HTTP and ingress policies'),
                ('webd','Test native webd staged import transport limits')]:
                self.assertTrue(required_staged_http_gate(jobs[name],kind))
                for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                    with self.subTest(job=name,kind=kind,mutation=mutation):
                        job=deepcopy(jobs[name])
                        step=next(value for value in job['steps'] if value.get('name')==label)
                        if mutation=='selector':
                            step['run']=step['run'].replace('memory::import_upload::ingress::tests::' if kind=='clawd'
                                else 'request_limits::memory_import_tests::','missing::transport::')
                        elif mutation=='listing':
                            step['run']=step['run'].replace('--list --color never','--list')
                        elif mutation=='require':
                            step['run']=re.sub(r'--require [A-Za-z0-9_:]+','--require absent_case',step['run'],count=1)
                        elif mutation=='ignore':
                            step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                        elif mutation=='conditional':
                            step['if']='false'
                        elif mutation=='continue':
                            step['continue-on-error']=True
                        else:
                            job['steps'].remove(step)
                            if mutation=='late':job['steps'].append(step)
                            else:
                                prior=('Test staged memory whole preview confirmation and privacy lifecycle' if kind=='clawd'
                                    else 'Test bounded staged import HTTP and ingress policies')
                                previous=next(value for value in job['steps'] if value.get('name')==prior)
                                job['steps'].insert(job['steps'].index(previous),step)
                        self.assertFalse(required_staged_http_gate(job,kind))
    def test_shared_and_whole_import_gates_keep_all_atomicity_and_privacy_cases(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            for kind,label,prefix in [
                ('shared','Test shared memory import filtering and canonical insertion','memory::ux::import_items::tests::'),
                ('whole','Test staged memory whole preview confirmation and privacy lifecycle','memory::import_upload::flow_tests::')]:
                self.assertTrue(required_staged_import_integration_gate(jobs[name],kind))
                for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                    with self.subTest(job=name,kind=kind,mutation=mutation):
                        job=deepcopy(jobs[name])
                        step=next(value for value in job['steps'] if value.get('name')==label)
                        if mutation=='selector':
                            step['run']=step['run'].replace(prefix,'missing::integration::')
                        elif mutation=='listing':
                            step['run']=step['run'].replace('--list --color never','--list')
                        elif mutation=='require':
                            step['run']=re.sub(r'--require [A-Za-z0-9_:]+','--require absent_case',step['run'],count=1)
                        elif mutation=='ignore':
                            step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                        elif mutation=='conditional':
                            step['if']='false'
                        elif mutation=='continue':
                            step['continue-on-error']=True
                        else:
                            job['steps'].remove(step)
                            if mutation=='late':
                                job['steps'].append(step)
                            else:
                                prior_name=('Test bounded streaming memory export decoding' if kind=='shared'
                                            else 'Test shared memory import filtering and canonical insertion')
                                prior=next(value for value in job['steps'] if value.get('name')==prior_name)
                                job['steps'].insert(job['steps'].index(prior),step)
                        self.assertFalse(required_staged_import_integration_gate(job,kind))
    def test_streaming_import_gate_keeps_all_unicode_checksum_and_io_cases(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_streaming_import_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(value for value in job['steps'] if value.get('name')=='Test bounded streaming memory export decoding')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::import_upload::parser::tests::','missing::parser::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require consumer_failure_keeps_its_original_machine_error','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        if mutation=='late':
                            job['steps'].append(step)
                        else:
                            prior=next(value for value in job['steps'] if value.get('name')=='Test durable memory import upload foundations')
                            job['steps'].insert(job['steps'].index(prior),step)
                    self.assertFalse(required_streaming_import_gate(job))
    def test_import_upload_gate_requires_all_storage_lease_and_reader_cases(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_import_upload_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(value for value in job['steps'] if value.get('name')=='Test durable memory import upload foundations')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::import_upload::storage_tests::','missing::upload::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require schema::missing_schema_read_never_creates_tables','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        if mutation=='late':
                            job['steps'].append(step)
                        else:
                            prior=next(value for value in job['steps'] if value.get('name')=='Test project lookup isolation and read-only identity contracts')
                            job['steps'].insert(job['steps'].index(prior),step)
                    self.assertFalse(required_import_upload_gate(job))

    def test_project_read_gate_preserves_normal_reads_and_all_new_contracts(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_project_read_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test project lookup isolation and read-only identity contracts')
                    if mutation=='selector':
                        step['run']=step['run'].replace('http::memory_project_read_tests::','missing::project_read::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require principal_reassigned_before_project_read','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        if mutation=='late': job['steps'].append(step)
                        else:
                            prior=next(value for value in job['steps'] if value.get('name')=='Test memory read boundary authentication contracts')
                            job['steps'].insert(job['steps'].index(prior),step)
                    self.assertFalse(required_project_read_gate(job))

    def test_memory_read_boundary_gate_retains_every_route_and_normal_control(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_memory_read_boundary_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test memory read boundary authentication contracts')
                    if mutation=='selector':
                        step['run']=step['run'].replace('http::memory_read_boundary_tests::','missing::read_boundary::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require settings::role_downgraded','--require settings::absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        if mutation=='late': job['steps'].append(step)
                        else:
                            prior=next(value for value in job['steps'] if value.get('name')=='Test capacity read authentication snapshot contracts')
                            job['steps'].insert(job['steps'].index(prior),step)
                    self.assertFalse(required_memory_read_boundary_gate(job))

    def test_capacity_read_authentication_gate_runs_after_complete_old_memory_HTTP(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_capacity_read_authentication_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','early','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test capacity read authentication snapshot contracts')
                    if mutation=='selector':
                        step['run']=step['run'].replace('http::memory_capacity_read_race_tests::','missing::read_race_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require principal_reassigned_after_authentication_does_not_return_previous_owner_state','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        if mutation=='late': job['steps'].append(step)
                        else:
                            prior=next(value for value in job['steps'] if value.get('name')=='Test memory vector status policy and scoped controls')
                            job['steps'].insert(job['steps'].index(prior),step)
                    self.assertFalse(required_capacity_read_authentication_gate(job))

    def test_lazy_domain_sampling_gate_preserves_all_required_native_cases(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_lazy_domain_sampling_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test lazy memory domain sampling and fresh observations')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::availability::sampling_tests::','missing::sampling_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require failed_actual_measurement_is_shared_once_and_a_later_read_can_recover','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        job['steps'].append(step)
                    self.assertFalse(required_lazy_domain_sampling_gate(job))

    def test_manual_growth_observation_and_authenticated_http_gate_is_required(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_manual_growth_observation_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test manual memory growth availability and read-only HTTP projection')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::ux::growth_availability_tests::','missing::growth_availability_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require invalid_budget_is_visible_but_not_normalized_by_http_read','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        job['steps'].append(step)
                    self.assertFalse(required_manual_growth_observation_gate(job))

    def test_configured_budget_atomicity_and_source_quota_gate_cannot_be_skipped(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_budget_policy_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test configured memory budget and source-only quota contracts')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::ux::budget_policy_tests::','missing::budget_policy_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require failed_refresh_rolls_back_pressure_and_budget_together','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        job['steps'].append(step)
                    self.assertFalse(required_budget_policy_gate(job))

    def test_growth_state_repair_and_releasing_controls_are_required(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_growth_state_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test invalid capacity state admission and recovery controls')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::ux::growth_state_tests::','missing::growth_state_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require missing_pressure_correction_fails_closed_without_repairing_the_observation','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        job['steps'].append(step)
                    self.assertFalse(required_growth_state_gate(job))

    def test_domain_growth_and_releasing_controls_cannot_be_skipped(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_domain_budget_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue','late']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test actual memory domain budget and effect contracts')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::ux::budget_contract_tests::','missing::budget_contract_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require growth::actual_domain_budget_rejects_private_correction_growth','--require growth::absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    elif mutation=='continue':
                        step['continue-on-error']=True
                    else:
                        job['steps'].remove(step)
                        job['steps'].append(step)
                    self.assertFalse(required_domain_budget_gate(job))

    def test_reindex_startup_and_rejection_controls_are_required_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_reindex_startup_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test rejected reindex startup and no-side-effect contracts')
                    if mutation=='selector':
                        step['run']=step['run'].replace('embedding_jobs::rejected_startup_tests::','missing::rejected_startup_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require remote_reindex_requires_explicit_consent_before_outbound_jobs_exist','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_reindex_startup_gate(job))

    def test_retained_memory_vector_policy_and_status_cannot_be_skipped(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_retained_memory_vector_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test retained memory vector policy and read-only status fences')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory_routes::vector_policy_tests::','missing::vector_policy_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require index_policy_status_still_requires_authentication','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_retained_memory_vector_gate(job))

    def test_remaining_memory_writer_gate_requires_every_case_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_remaining_memory_writer_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test remaining memory write authority and policy fences')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory_routes::write_fence_tests::','missing::write_fence_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require changed_original_role_cannot_commit_settings','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_remaining_memory_writer_gate(job))

    def test_memory_actor_gate_requires_every_case_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_memory_actor_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test original request memory mutation authority')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory_routes::actor_fence_tests::','missing::actor_fence_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require changed_original_role_is_rejected_at_the_actual_writer','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_memory_actor_gate(job))

    def test_snapshot_maintenance_gate_requires_every_case_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_snapshot_maintenance_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test terminal snapshot build maintenance')
                    if mutation=='selector':
                        step['run']=step['run'].replace('capacity_http_tests::snapshot_maintenance::','missing::snapshot_maintenance::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require healthy_paused_build_poll_stays_read_only_under_an_independent_writer','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_snapshot_maintenance_gate(job))

    def test_embedding_commit_gate_requires_every_case_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_embedding_commit_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test embedding response publication and recovery')
                    if mutation=='selector':
                        step['run']=step['run'].replace('capacity_http_tests::response_commit::','missing::response_commit::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace('--list --color never','--list')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require canonical_content_is_rechecked_even_if_its_index_row_did_not_change','--require absent_case')
                    elif mutation=='ignore':
                        step['run']=step['run'].replace('--test-threads=1 --nocapture','--test-threads=1 --nocapture || true')
                    elif mutation=='conditional':
                        step['if']='false'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_embedding_commit_gate(job))

    def test_inflight_embedding_gate_requires_every_case_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_inflight_embedding_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test inflight embedding recall and lease fences')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::embedding_jobs::capacity_http_tests::recall_races::','memory::embedding_jobs::missing_races::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace(' | python3 scripts/tests/assert_rust_test_selection.py',' | true')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require recall_suppression_after_request_cannot_reactivate_a_late_vector','')
                    elif mutation=='ignore':
                        step['run']+='\ntrue || true\n'
                    elif mutation=='conditional':
                        step['if']='${{ always() }}'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_inflight_embedding_gate(job))

    def test_projection_transaction_gate_requires_every_case_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_projection_transaction_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test projection transactions without losing parent ownership')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::projection_transaction::tests::','memory::transaction_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace(' | python3 scripts/tests/assert_rust_test_selection.py',' | true')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require standalone_failure_rolls_back_and_releases_the_writer','')
                    elif mutation=='ignore':
                        step['run']+='\ntrue || true\n'
                    elif mutation=='conditional':
                        step['if']='${{ always() }}'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_projection_transaction_gate(job))

    def test_recall_gate_requires_all_cases_and_refuses_bypasses_on_both_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_recall_memory_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional','continue']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test recall mutation indexes without hiding failures')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::ux::recall_contract_tests::','memory::recall_contract_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace(' | python3 scripts/tests/assert_rust_test_selection.py',' | true')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require corrections::fact_correction_replaces_index_and_fts_without_retaining_old_content','')
                    elif mutation=='ignore':
                        step['run']+='\ntrue || true\n'
                    elif mutation=='conditional':
                        step['if']='${{ always() }}'
                    else:
                        step['continue-on-error']=True
                    self.assertFalse(required_recall_memory_gate(job))

    def test_feedback_gate_requires_every_actual_case_on_both_native_platforms(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_feedback_memory_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if step.get('name')=='Test feedback mutation atomicity without hiding failures')
                    if mutation=='selector':
                        step['run']=step['run'].replace('memory::ux::feedback_tests::','memory::feedback_tests::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace(' | python3 scripts/tests/assert_rust_test_selection.py',' | true')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require failures::incorrect_without_content_does_not_commit_feedback','')
                    elif mutation=='ignore':
                        step['run']+='\ntrue || true\n'
                    else:
                        step['if']='${{ always() }}'
                    self.assertFalse(required_feedback_memory_gate(job))

    def test_builtin_memory_selection_cannot_succeed_with_zero_or_missing_cases(self):
        jobs=load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux','polyglot-skill-macos']:
            self.assertTrue(required_builtin_memory_gate(jobs[name]))
            for mutation in ['selector','listing','require','ignore','conditional']:
                with self.subTest(job=name,mutation=mutation):
                    job=deepcopy(jobs[name])
                    step=next(step for step in job['steps'] if 'skills::builtin::builtin_memory_store:: -- --list' in step.get('run',''))
                    if mutation=='selector':
                        step['run']=step['run'].replace('skills::builtin::builtin_memory_store::','skills::builtin_memory_store::')
                    elif mutation=='listing':
                        step['run']=step['run'].replace(' | python3 scripts/tests/assert_rust_test_selection.py',' | true')
                    elif mutation=='require':
                        step['run']=step['run'].replace('--require conversation_scope_isolated_and_child_mutation_is_denied','')
                    elif mutation=='ignore':
                        step['run']+='\ntrue || true\n'
                    else:
                        step['if']='${{ always() }}'
                    self.assertFalse(required_builtin_memory_gate(job))

    def test_memory_quota_accounting_gates_both_native_runtime_jobs(self):
        jobs = load('macos-polyglot-skill-ci.yml')['jobs']
        for name in ['runtime-read-linux', 'polyglot-skill-macos']:
            for selector in ['memory::retention', 'memory::']:
                command = f'cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture'
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ['remove', 'ignore', 'continue', 'conditional']:
                    with self.subTest(job=name, selector=selector, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job['steps'] if command in step.get('run', ''))
                        if mutation == 'remove': step['run'] = step['run'].replace(command, 'true')
                        elif mutation == 'ignore': step['run'] = step['run'].replace(command, command + ' || true')
                        elif mutation == 'continue': step['continue-on-error'] = True
                        else: step['if'] = '${{ always() }}'
                        self.assertFalse(required_command_gate(job, command))
        artifact = next(step for step in jobs['runtime-read-linux']['steps']
                        if step.get('name') == 'Build the exact verified Linux core without publishing a release')
        self.assertIn('memory_quota_accounting', artifact['run'])

    def test_principal_identity_lifecycle_gates_both_native_runtime_jobs(self):
        jobs = load('macos-polyglot-skill-ci.yml')['jobs']
        command = 'cargo test --locked -p clawd --bin clawd identity_lifecycle -- --test-threads=1 --nocapture'
        retained = [
            'cargo test --locked -p clawd --bin clawd repo::auth:: -- --test-threads=1 --nocapture --skip identity_lifecycle',
            'cargo test --locked -p clawd --bin clawd principal_ownership -- --test-threads=1 --nocapture',
        ]
        for name in ['runtime-read-linux', 'polyglot-skill-macos']:
            self.assertTrue(required_command_gate(jobs[name], command))
            for retained_command in retained:
                self.assertTrue(required_command_gate(jobs[name], retained_command))
            for mutation in ['remove', 'ignore', 'continue', 'conditional']:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job['steps'] if command in step.get('run', ''))
                    if mutation == 'remove': step['run'] = step['run'].replace(command, 'true')
                    elif mutation == 'ignore': step['run'] = step['run'].replace(command, command + ' || true')
                    elif mutation == 'continue': step['continue-on-error'] = True
                    else: step['if'] = '${{ always() }}'
                    self.assertFalse(required_command_gate(job, command))
        artifact = next(step for step in jobs['runtime-read-linux']['steps']
                        if step.get('name') == 'Build the exact verified Linux core without publishing a release')
        self.assertIn('principal_identity_lifecycle', artifact['run'])

    def test_artifact_digest_targets_gate_native_unix_and_signed_payloads(self):
        jobs = load('macos-polyglot-skill-ci.yml')['jobs']
        command = 'cargo test --locked -p agent-skill-sdk receipt::artifact_read_tests -- --test-threads=16 --nocapture'
        for name in ['runtime-read-linux', 'polyglot-skill-macos']:
            self.assertTrue(required_command_gate(jobs[name], command))
            for mutation in ['remove', 'ignore', 'continue', 'conditional']:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job['steps'] if command in step.get('run', ''))
                    if mutation == 'remove': step['run'] = step['run'].replace(command, 'true')
                    elif mutation == 'ignore': step['run'] = step['run'].replace(command, command + ' || true')
                    elif mutation == 'continue': step['continue-on-error'] = True
                    else: step['if'] = '${{ always() }}'
                    self.assertFalse(required_command_gate(job, command))
        for name in ['Build the exact verified Linux core without publishing a release',
                     'Build exact Linux STT consumers and verified skill receipt']:
            step = next(step for step in jobs['runtime-read-linux']['steps'] if step.get('name') == name)
            self.assertIn('artifact_regular_file_reads', step['run'])

    def test_windows_artifact_digest_gate_is_native_and_not_optional(self):
        job = load('macos-polyglot-skill-ci.yml')['jobs']['skill-sdk-metadata-windows']
        self.assertEqual(job['runs-on'], 'windows-2022')
        command = 'cargo test --locked -p agent-skill-sdk --lib receipt::artifact_read_tests -- --test-threads=4 --nocapture'
        self.assertTrue(required_command_gate(job, command))
        for mutation in ['remove', 'ignore', 'continue', 'conditional']:
            with self.subTest(mutation=mutation):
                changed = deepcopy(job)
                step = next(step for step in changed['steps'] if command in step.get('run', ''))
                if mutation == 'remove': step['run'] = step['run'].replace(command, 'true')
                elif mutation == 'ignore': step['run'] = step['run'].replace(command, command + ' || true')
                elif mutation == 'continue': step['continue-on-error'] = True
                else: step['if'] = '${{ always() }}'
                self.assertFalse(required_command_gate(changed, command))

    def test_windows_metadata_descriptor_gate_is_native_and_not_optional(self):
        job = load("macos-polyglot-skill-ci.yml")["jobs"]["skill-sdk-metadata-windows"]
        self.assertEqual(job['runs-on'], 'windows-2022')
        self.assertNotIn('continue-on-error', job)
        self.assertEqual(job['if'], "${{ github.event_name != 'workflow_dispatch' || inputs.verification_scope != 'ui' }}")
        command = 'cargo test --locked -p agent-skill-sdk --lib metadata_file::tests -- --test-threads=4 --nocapture'
        self.assertTrue(required_command_gate(job, command))
        for mutation in ['remove', 'ignore', 'continue', 'conditional']:
            with self.subTest(mutation=mutation):
                changed = deepcopy(job)
                step = next(step for step in changed['steps'] if command in step.get('run', ''))
                if mutation == 'remove': step['run'] = step['run'].replace(command, 'true')
                elif mutation == 'ignore': step['run'] = step['run'].replace(command, command + ' || true')
                elif mutation == 'continue': step['continue-on-error'] = True
                else: step['if'] = '${{ always() }}'
                self.assertFalse(required_command_gate(changed, command))

    def test_sdk_regular_metadata_reads_gate_both_native_systems_and_signed_payloads(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        command = "cargo test --locked -p agent-skill-sdk receipt::read_tests -- --test-threads=16 --nocapture"
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            self.assertTrue(required_command_gate(jobs[name], command))
            self.assertTrue(required_command_gate(jobs[name],
                "cargo test --locked -p agent-skill-sdk metadata_file::tests -- --test-threads=16 --nocapture"))
            for mutation in ["remove", "ignore", "continue", "conditional"]:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job["steps"] if command in step.get("run", ""))
                    if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                    elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                    elif mutation == "continue": step["continue-on-error"] = True
                    else: step["if"] = "${{ always() }}"
                    self.assertFalse(required_command_gate(job, command))
        for name in ["Build the exact verified Linux core without publishing a release",
                     "Build exact Linux STT consumers and verified skill receipt"]:
            step = next(step for step in jobs['runtime-read-linux']['steps'] if step.get('name') == name)
            self.assertIn('receipt_regular_file_reads', step['run'])

    def test_sdk_publication_privileged_cases_gate_both_native_systems_and_signed_payloads(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for command in [
                'cargo test --locked -p agent-skill-sdk --lib --no-run --message-format=json > "$RUNNER_TEMP/sdk-publication-tests.json"',
                'python3 scripts/tests/run_privileged_secure_fs_case.py "$RUNNER_TEMP/sdk-publication-tests.json" --sdk-publication',
            ]:
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if command in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                        elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_command_gate(job, command))
        for name in ["Build the exact verified Linux core without publishing a release",
                     "Build exact Linux STT consumers and verified skill receipt"]:
            step = next(step for step in jobs['runtime-read-linux']['steps'] if step.get('name') == name)
            self.assertIn('receipt_publication_metadata', step['run'])

    def test_sdk_publication_metadata_is_mandatory_on_both_native_platforms(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        command = "cargo test --locked -p agent-skill-sdk receipt::publication_tests -- --test-threads=16 --nocapture"
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            self.assertTrue(required_command_gate(jobs[name], command))
            for mutation in ["remove", "ignore", "continue", "conditional"]:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job["steps"] if command in step.get("run", ""))
                    if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                    elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                    elif mutation == "continue": step["continue-on-error"] = True
                    else: step["if"] = "${{ always() }}"
                    self.assertFalse(required_command_gate(job, command))

    def test_full_stt_suite_is_mandatory_on_both_native_platforms(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for command in [
                "cargo test --locked -p audio-transcribe-skill -- --test-threads=16 --nocapture",
                "cargo test --locked -p claw-core llm_connection_environment::tests -- --test-threads=1 --nocapture",
                "cargo test --locked -p clawd --bin clawd selected_connection_tests -- --test-threads=1 --nocapture",
            ]:
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, command=command, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if command in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                        elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_command_gate(job, command))

    def test_stt_runner_and_skill_are_retained_together_with_attested_receipts(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["runtime-read-linux"]["steps"]
        named = {step.get("name"): step for step in steps}
        build = named["Build exact Linux STT consumers and verified skill receipt"]
        attest = named["Attest exact Linux STT consumers and skill package"]
        upload = named["Preserve exact verified Linux STT consumers and skill package"]
        self.assertNotIn("if", build)
        self.assertNotIn("continue-on-error", build)
        self.assertIn("set -euo pipefail", build["run"])
        for fragment in [
            "-p skill-runner --bin skill-runner", "-p audio-transcribe-skill --bin audio-transcribe-skill",
            "-p agent-skill-sdk --bin skillctl", "scripts/project_skill_receipts.py --skill audio_transcribe",
            "source-commit", "verified-gates", "SHA256SUMS",
        ]:
            self.assertIn(fragment, build["run"])
        self.assertLess(steps.index(build), steps.index(attest))
        self.assertLess(steps.index(attest), steps.index(upload))
        self.assertEqual(attest["if"], "${{ github.event_name != 'pull_request' }}")
        self.assertTrue(attest["uses"].startswith("actions/attest-build-provenance@"))
        self.assertIn("runtime-stt-${{ github.sha }}.tar.gz", attest["with"]["subject-path"])
        self.assertEqual(upload["with"]["if-no-files-found"], "error")

    def test_catalogue_selection_and_cli_contracts_gate_both_systems_and_signed_artifact(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        commands = [
            'cargo test --locked -p claw-core llm_provider_descriptors::tests -- --test-threads=1 --nocapture',
            'cargo test --locked -p clawd --bin clawd llm_gateway::tests -- --test-threads=1 --nocapture',
            'cargo test --locked -p clawd --bin clawd llm_provider_runtime_tests -- --test-threads=1 --nocapture',
            "cargo test --locked -p clawd --bin clawd model_catalog -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawd --bin clawd task_debug_trace_tests -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawd --bin clawd task_model_selection -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawcli commands::models_tests -- --test-threads=1 --nocapture",
        ]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for command in commands:
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, command=command, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if command in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                        elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_command_gate(job, command))
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        for fragment in [
            "cargo build --locked --release -p clawd --bin clawd -p clawcli --bin clawcli",
            'install -m 755 target/release/clawcli "$artifact_root/runtime-read/clawcli"',
            "sha256sum clawd clawcli source-commit verified-gates",
            "model_catalog_truthfulness task_model_selection cli_model_readiness",
            "provider_descriptors",
        ]:
            self.assertIn(fragment, build["run"])

    def test_effective_provider_credentials_gate_both_systems_without_skipping_or_ignoring_failures(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        selector = "provider_credential_tests"
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            self.assertTrue(required_reload_gate(jobs[name], selector))
            for mutation in ["remove", "ignore", "continue", "conditional"]:
                with self.subTest(job=name, mutation=mutation):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job["steps"] if f"--bin clawd {selector} --" in step.get("run", ""))
                    if mutation == "remove": step["run"] = step["run"].replace(f"--bin clawd {selector} --", "--bin clawd absent_fixture --")
                    elif mutation == "ignore": step["run"] = step["run"].replace(f"{selector} -- --test-threads=1", f"{selector} -- --ignored --test-threads=1")
                    elif mutation == "continue": step["continue-on-error"] = True
                    else: step["if"] = "${{ always() }}"
                    self.assertFalse(required_reload_gate(job, selector))
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        self.assertIn("provider_credentials", build["run"])

    def test_configuration_lock_and_credential_transaction_suites_gate_both_systems(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for command in [
                "cargo test --locked -p clawd --bin clawd configuration_files::tests -- --test-threads=1 --nocapture",
                "cargo test --locked -p clawd --bin clawd channel_registration_race_tests::application_tests -- --test-threads=1 --nocapture",
                "cargo test --locked -p claw-core secrets::model_environment::tests -- --test-threads=1 --nocapture",
            ]:
                self.assertTrue(required_command_gate(jobs[name], command))
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, command=command, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if command in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(command, "true")
                        elif mutation == "ignore": step["run"] = step["run"].replace(command, command + " || true")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_command_gate(job, command))

    def test_shared_configuration_and_retained_credentials_gate_both_native_systems(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for selector in ["configuration_write_tests", "runtime_config_write_", "llm_credentials_"]:
                self.assertTrue(required_reload_gate(jobs[name], selector), (name, selector))
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        self.assertIn("configuration_publication", build["run"])

    def test_configuration_write_gate_cannot_skip_or_discard_a_failure(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for selector in ["configuration_write_tests", "runtime_config_write_", "llm_credentials_"]:
                for mutation in ["remove", "ignore", "continue", "conditional"]:
                    with self.subTest(job=name, selector=selector, mutation=mutation):
                        job = deepcopy(jobs[name])
                        step = next(step for step in job["steps"] if f"--bin clawd {selector} --" in step.get("run", ""))
                        if mutation == "remove": step["run"] = step["run"].replace(f"--bin clawd {selector} --", "--bin clawd absent_fixture --")
                        elif mutation == "ignore": step["run"] = step["run"].replace(f"{selector} -- --test-threads=1", f"{selector} -- --ignored --test-threads=1")
                        elif mutation == "continue": step["continue-on-error"] = True
                        else: step["if"] = "${{ always() }}"
                        self.assertFalse(required_reload_gate(job, selector))

    def test_runtime_reload_cases_gate_both_native_systems_and_the_signed_core(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            self.assertTrue(required_reload_gate(jobs[name]), name)
        build = next(step for step in jobs["runtime-read-linux"]["steps"]
                     if step.get("name") == "Build the exact verified Linux core without publishing a release")
        self.assertIn("skill_reload_publication", build["run"])

    def test_runtime_reload_gate_cannot_skip_ignore_or_discard_failures(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        for name in ["runtime-read-linux", "polyglot-skill-macos"]:
            for mode in ["remove", "ignore", "continue", "conditional"]:
                with self.subTest(job=name, mutation=mode):
                    job = deepcopy(jobs[name])
                    step = next(step for step in job["steps"] if "--bin clawd skill_reload_tests" in step.get("run", ""))
                    if mode == "remove": step["run"] = step["run"].replace("--bin clawd skill_reload_tests", "--bin clawd absent_fixture")
                    elif mode == "ignore": step["run"] = step["run"].replace("skill_reload_tests -- --test-threads=1 --nocapture", "skill_reload_tests -- --ignored --test-threads=1 --nocapture")
                    elif mode == "continue": step["continue-on-error"] = True
                    else: step["if"] = "${{ always() }}"
                    self.assertFalse(required_reload_gate(job))

    def test_supported_runners_preserve_architecture_and_deployment_floor(self):
        for name, job_id, runner, target in EXPECTED:
            with self.subTest(workflow=name, job=job_id):
                self.assertTrue(valid_job(load(name)["jobs"][job_id], runner, target))

    def test_retired_runner_wrong_architecture_and_newer_floor_are_rejected(self):
        for name, job_id, runner, target in EXPECTED:
            job = load(name)["jobs"][job_id]
            for changed_runner in [runner.replace("macos-15", "macos-14"), "macos-15-intel"]:
                changed = deepcopy(job)
                changed["runs-on"] = changed_runner
                with self.subTest(workflow=name, runner=changed_runner):
                    self.assertFalse(valid_job(changed, runner, target))
            if target:
                changed = deepcopy(job)
                changed["env"]["MACOSX_DEPLOYMENT_TARGET"] = "15.0"
                with self.subTest(workflow=name, mutation="deployment_floor"):
                    self.assertFalse(valid_job(changed, runner, target))

    def test_runner_contract_is_in_native_ci(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = "python3 scripts/nl_tests/macos_runner_workflow_tests.py"
        self.assertEqual(sum(command in step.get("run", "") for step in steps), 1)

    def test_focused_ui_mode_cannot_disable_automatic_full_runtime_checks(self):
        workflow = load("macos-polyglot-skill-ci.yml")
        option = workflow["on"]["workflow_dispatch"]["inputs"]["verification_scope"]
        self.assertEqual(option["default"], "all")
        self.assertEqual(option["options"], ["all", "ui"])
        self.assertEqual(workflow["jobs"]["polyglot-skill-macos"]["if"],
                         "${{ github.event_name != 'workflow_dispatch' || inputs.verification_scope != 'ui' }}")
        focused = workflow["jobs"]["ui-browser-reliability"]
        self.assertEqual(focused["if"],
                         "${{ github.event_name == 'workflow_dispatch' && inputs.verification_scope == 'ui' }}")
        self.assertEqual(focused["uses"], "./.github/workflows/ui-browser-reliability-ci.yml")
        lightweight = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]["steps"]
        guard = next(step for step in lightweight if step.get("name") == "Require unchanged runtime sources for manual UI-only verification")
        self.assertEqual(guard["if"], "${{ github.event_name == 'workflow_dispatch' && !inputs.full_runtime_validation }}")
        self.assertIn("git diff --quiet origin/main HEAD -- Cargo.toml Cargo.lock .cargo crates optional_skills external_skills configs docker prompts pi_app services", guard["run"])
        self.assertIn("exit 1", guard["run"])

    def test_full_mode_runs_both_ui_platforms_alongside_every_runtime_gate(self):
        workflow = load("macos-polyglot-skill-ci.yml")
        jobs = workflow["jobs"]
        full = jobs["full-ui-browser-reliability"]
        self.assertEqual(full["uses"], "./.github/workflows/ui-browser-reliability-ci.yml")
        self.assertEqual(full["with"], {"full_runtime_validation": True})
        for job_id in ["skill-sdk-linux", "runtime-read-linux", "media-session-portability", "polyglot-skill-macos"]:
            self.assertEqual(full["if"], jobs[job_id]["if"])
        self.assertNotIn("continue-on-error", full)
        native = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]
        self.assertEqual(native["strategy"]["matrix"]["os"], ["ubuntu-22.04", "macos-15"])
        self.assertFalse(native["strategy"]["fail-fast"])

    def test_lightweight_entry_points_cannot_select_the_internal_full_runtime_mode(self):
        lightweight = load("ui-browser-reliability-ci.yml")
        self.assertIsNone(lightweight["on"]["workflow_dispatch"])
        option = lightweight["on"]["workflow_call"]["inputs"]["full_runtime_validation"]
        self.assertEqual(option["type"], "boolean")
        self.assertIs(option["default"], False)
        self.assertIs(option["required"], False)
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        self.assertNotIn("with", jobs["ui-browser-reliability"])
        self.assertEqual([name for name, job in jobs.items()
                          if job.get("with", {}).get("full_runtime_validation") is True],
                         ["full-ui-browser-reliability"])

    def test_browser_preparation_is_followed_by_the_runtime_signature_selector(self):
        steps = load("macos-arm64-release-artifact.yml")["jobs"]["build-macos"]["steps"]
        script = next(step["run"] for step in steps if step.get("name") == "Test signed system browser and isolated skill profiles")
        prepare = script.index("python3 scripts/prepare_macos_ci_browser.py")
        self.assertLess(prepare, script.index('executable="$(bash scripts/macos-system-browser.sh --resolve-executable)"'))

    def test_runtime_reads_have_independent_cache_and_attested_core_artifacts(self):
        jobs = load("macos-polyglot-skill-ci.yml")["jobs"]
        sdk = jobs["skill-sdk-linux"]
        runtime = jobs["runtime-read-linux"]
        self.assertEqual(runtime["if"], jobs["polyglot-skill-macos"]["if"])
        self.assertEqual(runtime["runs-on"], "ubuntu-22.04")
        self.assertNotIn("cargo test --locked -p clawd", "\n".join(step.get("run", "") for step in sdk["steps"]))
        steps = runtime["steps"]
        named = {step.get("name"): step for step in steps}
        gate = named["Verify Linux indexed history reads and startup upgrades"]
        for selector in ["task_event_transport", "auth_contract_tests", "model_catalog", "model_settings_integrity_tests", "configuration_read_tests", "secure_workspace_fs", "aipp", "run_skill", "skill_admission::tests::state_mutations"]:
            self.assertIn(f"cargo test --locked -p clawd --bin clawd {selector}", gate["run"])
        native_steps = jobs["polyglot-skill-macos"]["steps"]
        stress_command = "cargo test --locked -p claw-core model_catalog::tests -- --test-threads=16"
        self.assertIn(stress_command, gate["run"])
        self.assertIn("for attempt in 1 2 3 4 5; do", gate["run"])
        native_stress = [step for step in native_steps if stress_command in step.get("run", "")]
        self.assertEqual(len(native_stress), 1)
        self.assertIn("for attempt in 1 2 3 4 5; do", native_stress[0]["run"])
        auth_command = "cargo test --locked -p clawd --bin clawd auth_contract_tests"
        self.assertEqual(sum(auth_command in step.get("run", "") for step in native_steps), 1)
        catalog_command = "cargo test --locked -p clawd --bin clawd model_catalog -- --test-threads=1 --nocapture"
        self.assertIn(catalog_command, gate["run"])
        self.assertEqual(sum(catalog_command in step.get("run", "") for step in native_steps), 1)
        self.assertNotIn("--bin clawd task_event_archive", gate["run"])
        self.assertIn("ripgrep", named["Prepare native build and sandbox dependencies"]["run"])
        self.assertIn("python3 scripts/check_aipp_decoupling.py --self-test", gate["run"])
        self.assertIn("python3 scripts/check_aipp_decoupling.py\n", gate["run"])
        build = named["Build the exact verified Linux core without publishing a release"]
        attest = named["Attest the exact Linux core provenance"]
        upload = named["Preserve the exact verified Linux core artifact"]
        self.assertLess(steps.index(gate), steps.index(build))
        self.assertLess(steps.index(named["Verify both product identities with the shared UI"]), steps.index(build))
        self.assertLess(steps.index(build), steps.index(attest))
        self.assertLess(steps.index(attest), steps.index(upload))
        self.assertIn("cargo build --locked --release -p clawd --bin clawd", build["run"])
        self.assertIn("$GITHUB_SHA", build["run"])
        self.assertIn("source-commit", build["run"])
        self.assertIn("aipp_decoupling", build["run"])
        self.assertIn("ui_auth_error_contract", build["run"])
        self.assertIn("model_catalog_fixture_isolation", build["run"])
        self.assertIn("aipp_read_surfaces", build["run"])
        self.assertIn("aipp_generation", build["run"])
        self.assertIn("model_catalog_read", build["run"])
        for selector in ["model_settings_integrity_tests", "configuration_read_tests", "secure_workspace_fs"]:
            command = f"cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1 --nocapture"
            self.assertIn(command, gate["run"])
            self.assertEqual(sum(command in step.get("run", "") for step in native_steps), 1)
        self.assertIn("--no-run --message-format=json", gate["run"])
        self.assertIn("scripts/tests/run_privileged_secure_fs_case.py", gate["run"])
        self.assertIn("model_settings_integrity", build["run"])
        self.assertIn("secure_workspace_metadata", build["run"])
        self.assertIn("configuration_reads", build["run"])
        mutation_command = "cargo test --locked -p clawd --bin clawd skill_admission::tests::state_mutations -- --test-threads=1 --nocapture"
        self.assertIn(mutation_command, gate["run"])
        self.assertEqual(sum(mutation_command in step.get("run", "") for step in native_steps), 1)
        self.assertIn("skill_state_mutations", build["run"])
        for selector in ["skill_admission::tests", "skill_switch_update_tests"]:
            command = f"cargo test --locked -p clawd --bin clawd {selector} -- --test-threads=1"
            self.assertIn(command, gate["run"])
            self.assertEqual(sum(command in step.get("run", "") for step in native_steps), 1)
        self.assertIn("skill_switch_updates", build["run"])
        self.assertIn("attest-build-provenance@", attest["uses"])
        self.assertIn("${{ github.sha }}", upload["with"]["name"])
        self.assertEqual(runtime["permissions"]["id-token"], "write")
        caches = [next(step["with"] for step in job["steps"] if "Swatinem/rust-cache@" in step.get("uses", "")) for job in [sdk, runtime]]
        self.assertNotEqual(caches[0]["key"], caches[1]["key"])
        for cache in caches:
            self.assertEqual(str(cache["cache-on-failure"]).lower(), "false")
            self.assertEqual(cache["save-if"], "${{ github.event_name != 'pull_request' }}")

    def test_deployable_ui_is_captured_before_desktop_build_and_uploaded_only_after_checks(self):
        steps = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]["steps"]
        named = {step.get("name"): step for step in steps}
        stage = named["Stage the production web UI artifact"]
        upload = named["Upload the verified production web UI"]
        self.assertLess(steps.index(named["Test, typecheck and build UI"]), steps.index(stage))
        self.assertLess(steps.index(stage), steps.index(named["Verify shared desktop UI transport and build"]))
        self.assertLess(steps.index(named["Stress QR scope commits without provider calls"]), steps.index(upload))
        self.assertEqual(stage["if"], "${{ runner.os == 'Linux' }}")
        self.assertEqual(upload["if"], stage["if"])
        self.assertIn("node UI/build-fingerprint.mjs UI configs/product_identity.toml", stage["run"])
        self.assertIn('"$GITHUB_SHA" > UI/dist/.agent-ui-source-commit', stage["run"])
        self.assertIn('-C UI dist', stage["run"])
        self.assertIn('sha256sum "$archive_name"', stage["run"])
        self.assertEqual(upload["with"]["name"], "verified-web-ui-${{ github.sha }}")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")
        self.assertIn("python3 scripts/nl_tests/macos_runner_workflow_tests.py",
                      named["Check browser preparation boundaries"]["run"])
        self.assertIn("--only-binary=:all: --requirement scripts/nl_tests/requirements.txt",
                      named["Check browser preparation boundaries"]["run"])

    def test_ui_authentication_browser_regressions_gate_both_native_artifacts(self):
        job = load("ui-browser-reliability-ci.yml")["jobs"]["ui-and-browser"]
        self.assertEqual(job["strategy"]["matrix"]["os"], ["ubuntu-22.04", "macos-15"])
        steps = job["steps"]
        gate = next(step for step in steps if step.get("name") == "Verify authentication ownership and transient recovery")
        self.assertNotIn("if", gate)
        self.assertIn("set -euo pipefail", gate["run"])
        for fixture in ["ownership", "concurrency", "recovery"]:
            self.assertIn(f"node UI/test/authentication-{fixture}.browser.mjs", gate["run"])
        upload = next(step for step in steps if step.get("name") == "Upload the verified production web UI")
        self.assertLess(steps.index(gate), steps.index(upload))


if __name__ == "__main__":
    unittest.main()
