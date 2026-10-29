"""Bind the discovery fixture to attested fields from one current-session query."""
import copy
import json
from pathlib import Path
import unittest

from manual_case_assertions import evaluate_expectations
from manual_case_assertions_tests import result_with_steps, capability_group_load_step


class LoaderQueryContractTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).parent / 'cases/nl_cases_loader_contract_20261029.txt'
        row = next(line for line in path.read_text().splitlines() if line.startswith('loader_contract|'))
        _, _, self.tags, _, expectation = row.split('|', 4)
        self.expectation = expectation.removeprefix('expect=')
        self.step = {'step_id': 'step_2', 'requested_action_type': 'call_capability',
            'requested_capability': 'task_control.list', 'executed_skill': 'task_control', 'status': 'ok',
            'observed_evidence': {'items': [
                {'field': 'extra.count', 'kind': 'number', 'excerpt': '0'},
                {'field': 'extra.field_value.scope.kind', 'kind': 'string', 'excerpt': 'current_session'},
                {'field': 'extra.field_value.scope.source', 'kind': 'string', 'excerpt': 'current_task_record'},
            ]}}

    def check(self, steps=None):
        text = json.dumps({'scope_kind': 'current_session', 'count': 0})
        value = result_with_steps(steps if steps is not None else [capability_group_load_step(), self.step], text=text)
        status, _ = evaluate_expectations(self.expectation, self.tags, value, 'succeeded', text,
                                         value['data']['result_json'], {})
        return status

    def test_nested_attested_fields_pass_without_expanding_the_extractor(self):
        self.assertEqual(self.check(), 'pass')

    def test_reply_alone_does_not_prove_execution(self):
        self.assertEqual(self.check([]), 'fail')

    def test_other_session_is_rejected(self):
        self.step['observed_evidence']['items'][1]['excerpt'] = 'accessible'
        self.assertEqual(self.check(), 'fail')

    def test_untrusted_scope_source_is_rejected(self):
        self.step['observed_evidence']['items'][2]['excerpt'] = 'user_supplied'
        self.assertEqual(self.check(), 'fail')

    def test_each_machine_field_is_required(self):
        for index in range(3):
            step = copy.deepcopy(self.step)
            step['observed_evidence']['items'].pop(index)
            self.assertEqual(self.check([step]), 'fail')

    def test_no_duplicate_or_unrelated_business_dispatch(self):
        self.assertEqual(self.check([self.step, copy.deepcopy(self.step)]), 'fail')
        other = copy.deepcopy(self.step)
        other.update(executed_skill='other_fixture', requested_capability='fixture.mutate')
        self.assertEqual(self.check([self.step, other]), 'fail')


if __name__ == '__main__':
    unittest.main()
