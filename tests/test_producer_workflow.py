"""Static checks of GitHub event wiring; hosted runners remain required."""
import unittest
from pathlib import Path

import yaml


class ProducerWorkflowTest(unittest.TestCase):
    def test_pr_and_manual_sha_matrix_and_permissions(self):
        text = Path('.github/workflows/producer-validation.yml').read_text()
        workflow = yaml.safe_load(text)
        events = workflow.get('on', workflow.get(True))
        self.assertIn('pull_request', events)
        self.assertTrue(events['workflow_dispatch']['inputs']['expected_sha']['required'])
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        job = workflow['jobs']['producer']
        self.assertIn("github.event_name == 'pull_request' && '[\"ubuntu-latest\",\"macos-14\"]'", job['strategy']['matrix']['runner'])
        self.assertIn("github.event_name == 'pull_request' && github.sha || inputs.expected_sha", job['env']['EXPECTED_SHA'])
        checkouts = [s for s in job['steps'] if s.get('uses', '').startswith('actions/checkout@')]
        self.assertEqual(len(checkouts), 2)
        for checkout in checkouts:
            self.assertEqual(checkout['with']['ref'], '${{ github.sha }}')
            self.assertFalse(checkout['with']['persist-credentials'])
        self.assertIn('test "$EXPECTED_SHA" = "$EVENT_SHA"', text)
        self.assertIn('github.event.pull_request.number || github.ref', workflow['concurrency']['group'])
        self.assertNotIn('pull_request_target', events)
        self.assertNotIn('${{ secrets.', text)


if __name__ == '__main__':
    unittest.main()
